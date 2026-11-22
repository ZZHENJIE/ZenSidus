//! Market Screener panel: a saved preset (a Finviz preset for the first
//! round plus an ask/bid exchange list and a shared minimum size) drives a
//! live per-exchange filter over the shared Alpaca quote stream. One table
//! renders per exchange; a symbol joins the table of exchange `E` when its
//! ask or bid side sits on `E` with a size meeting the preset's minimum.
//! Presets live in the shared settings (managed from the toolbar's settings
//! button); the selection is persisted per panel instance.

use std::collections::BTreeSet;

use anysdk::alpaca_stream::StockMessage;
use gpui_kit::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window,
    base::dock::{PanelEvent, PanelInfo, PanelState},
    base::{h_flex, v_flex},
    component::{
        ActiveTheme as _, button::Button, combobox::ComboboxEvent, searchable_list::SearchableVec,
        table::TableState,
    },
    div, px,
};

use crate::data::api::ScreenerExport;
use crate::data::stream::{ServerStatus, StreamConsumer, StreamHandle, Wish, connected};
use crate::panels::{Panel, panel_title, refresh_button};
use crate::tr;
use crate::utils::exchanges::exchange_label;
use crate::utils::settings::MarketScreenerPreset;
use crate::utils::state::AppState;
use crate::utils::{AppIcon, message, settings};
use crate::widgets::datatable::{self, ExchangeTableDelegate};
use crate::widgets::dialog::{MarketPresetDraft, open_confirm_dialog, open_market_preset_dialog};
use crate::widgets::picker::{PickerAction, StringPicker};

pub struct MarketScreenerPanel {
    focus_handle: FocusHandle,
    picker: StringPicker,
    /// One table per exchange of the selected preset, in preset order.
    tables: Vec<Entity<TableState<ExchangeTableDelegate>>>,
    /// The exchange codes `tables` was built from (aligned; the captions
    /// read from here).
    exchanges: Vec<String>,
    /// The selected Market Screener preset (persisted per instance).
    preset: Option<String>,
    /// The preset content as last loaded, so the settings observer can tell
    /// a rule edit from an unrelated settings change.
    loaded: Option<MarketScreenerPreset>,
    /// The symbol pool as last fetched (Finviz order, uppercased). Single
    /// source of truth for the wire subscription; the tables mirror it.
    pool: Vec<String>,
    /// Whether this tab is the active one in its tab group. A hidden panel
    /// unsubscribes from the stream and resubscribes when shown again.
    active: bool,
    /// The panel's stream subscription (wish, filter, pump, lifecycle).
    stream: StreamHandle<Self>,
    /// Monotonic fetch counter: a preset switch while a pool fetch is in
    /// flight invalidates its outcome.
    fetch_generation: u64,
    /// Whether the tables need a rebuild on the next render: a preset switch
    /// happens inside this window's update cycle, and `TableState::new`
    /// needs the window — reachable directly from `render`, not from here.
    pending_resync: bool,
}

impl Panel for MarketScreenerPanel {
    type DataEntity = Self;
    const NAME: &'static str = "MarketScreenerPanel";
    const ICON: AppIcon = AppIcon::Market;
    const TITLE_KEY: &'static str = "app.panels.market_screener";

    fn new(window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self {
        let focus_handle = cx.focus_handle();
        let persisted_preset = info
            .get("preset")
            .and_then(|value| value.as_str())
            .map(String::from);
        let preset = persisted_preset
            .filter(|name| preset_names(cx).contains(name))
            .or_else(|| preset_names(cx).first().cloned());

        let picker = StringPicker::new(preset_names(cx), preset.clone(), window, cx);

        // Picking a preset in the picker switches the panel to it.
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

        // Presets are shared settings; follow edits made elsewhere (another
        // Market Screener instance's dialog, a future settings page).
        cx.observe_global_in::<AppState>(window, |this, window, cx| {
            this.resync(window, cx);
        })
        .detach();

        let mut this = Self {
            focus_handle,
            picker,
            tables: Vec::new(),
            exchanges: Vec::new(),
            preset,
            loaded: None,
            pool: Vec::new(),
            active: true,
            stream: StreamHandle::new(cx),
            fetch_generation: 0,
            pending_resync: false,
        };
        this.resync(window, cx);
        this.stream.attach_pump(cx);
        this
    }

    /// Refetch the pool from the selected preset's Finviz source.
    fn refresh(entity: WeakEntity<Self::DataEntity>, cx: &mut App) {
        let _ = entity.update(cx, |this, cx| this.fetch(cx));
    }
}

impl StreamConsumer for MarketScreenerPanel {
    const STAGE: &'static str = "market";

    fn stream_handle(&mut self) -> &mut StreamHandle<Self> {
        &mut self.stream
    }

    /// Mirror the stream status: connected re-pulls the pool (the startup
    /// fetch was skipped silently while offline); disconnected drops the
    /// tables' connected flag (nothing shown may be trusted anymore). The
    /// hit log itself is history and survives untouched.
    fn on_status(&mut self, status: &ServerStatus, cx: &mut Context<Self>) {
        self.set_connected(matches!(status, ServerStatus::Connected), cx);
        if matches!(status, ServerStatus::Connected) {
            self.fetch(cx);
        }
    }

    fn apply_batch(&mut self, messages: Vec<StockMessage>, cx: &mut Context<Self>) {
        let quotes: Vec<_> = messages
            .into_iter()
            .filter_map(|message| match message {
                StockMessage::Quote(quote) => Some(quote),
                _ => None,
            })
            .collect();
        if quotes.is_empty() {
            return;
        }
        // The selected preset's min_size, read once per batch: every hit is
        // evaluated against the rule as it stood when the quote arrived.
        let min_size = self
            .preset
            .as_deref()
            .and_then(|name| {
                cx.global::<AppState>()
                    .settings
                    .market_screener
                    .presets
                    .get(name)
            })
            .map(|preset| preset.min_size)
            .unwrap_or(0);
        // One update per table per batch: the batch already collapses the
        // pump's scheduling; this collapses the per-message table updates.
        for table in &self.tables {
            table.update(cx, |state, cx| {
                let changed = quotes
                    .iter()
                    .any(|quote| state.delegate_mut().apply_quote(quote, min_size));
                if changed {
                    cx.notify();
                }
            });
        }
    }
}

/// Market preset titles in settings order (a `BTreeMap`, so "first" is
/// stable).
fn preset_names(cx: &App) -> Vec<String> {
    cx.global::<AppState>()
        .settings
        .market_screener
        .presets
        .keys()
        .cloned()
        .collect()
}

/// Ticker column of the market-screener export, located by header name (the
/// server pins the market export columns to No. + Ticker).
fn extract_tickers(result: &ScreenerExport) -> Vec<String> {
    let Some(col) = result
        .headers
        .iter()
        .position(|header| header.trim().eq_ignore_ascii_case("ticker"))
    else {
        return Vec::new();
    };
    result
        .rows
        .iter()
        .filter_map(|row| row.get(col))
        .map(|cell| cell.text.trim().to_string())
        .filter(|ticker| !ticker.is_empty())
        .collect()
}

impl MarketScreenerPanel {
    /// Bring the panel in line with the shared settings: follow the preset
    /// list (a deleted selection falls back to the first remaining), then
    /// diff the loaded snapshot — a changed exchange set rebuilds the
    /// tables, a changed Finviz source refetches the pool, and a pure
    /// min_size edit needs nothing beyond the next render (rules are
    /// evaluated there).
    fn resync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names = preset_names(cx);
        let synced = self
            .preset
            .clone()
            .filter(|name| names.contains(name))
            .or_else(|| names.first().cloned());
        let selection_changed = synced != self.preset;
        self.preset = synced;

        let content = self.preset.as_ref().and_then(|name| {
            cx.global::<AppState>()
                .settings
                .market_screener
                .presets
                .get(name)
                .cloned()
        });
        let exchanges_changed =
            self.loaded.as_ref().map(|p| &p.exchanges) != content.as_ref().map(|p| &p.exchanges);
        let source_changed = self.loaded.as_ref().map(|p| &p.finviz_preset)
            != content.as_ref().map(|p| &p.finviz_preset);
        self.loaded = content;

        if exchanges_changed {
            self.rebuild_tables(window, cx);
        }
        if exchanges_changed || selection_changed {
            // Fresh delegates (or a renamed preset) must re-learn the pool
            // and the preset whose rules they render by.
            self.apply_pool(cx);
        }
        if source_changed {
            self.fetch(cx);
        }
        self.picker.set_items(preset_names(cx), window, cx);
        self.picker.set_selected(self.preset.clone(), window, cx);
        self.sync_subscription(cx);
        cx.notify();
    }

    /// Build one table per exchange of the selected preset. The delegates
    /// start empty; the pool rides in via [`Self::apply_pool`].
    fn rebuild_tables(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let exchanges = self
            .loaded
            .as_ref()
            .map(|preset| preset.exchanges.clone())
            .unwrap_or_default();
        let connected = connected(cx);
        self.exchanges = exchanges.clone();
        self.tables = exchanges
            .into_iter()
            .map(|code| {
                datatable::new_table_state(cx, window, |_| {
                    ExchangeTableDelegate::new(code.clone(), connected)
                })
            })
            .collect();
    }

    /// Push the current pool into every table.
    fn apply_pool(&mut self, cx: &mut Context<Self>) {
        for table in &self.tables {
            table.update(cx, |state, cx| {
                state.delegate_mut().set_pool(&self.pool);
                cx.notify();
            });
        }
    }

    /// Publish this panel's pool to the hub with quotes opted in. While the
    /// tab is not the active one the set is empty, unsubscribing its share
    /// of the union.
    fn sync_subscription(&mut self, cx: &mut Context<Self>) {
        let wanted: BTreeSet<String> = if self.active {
            self.pool.iter().cloned().collect()
        } else {
            BTreeSet::new()
        };
        self.stream.set_wish(
            Wish {
                symbols: wanted,
                quotes: true,
                ..Default::default()
            },
            cx,
        );
    }

    /// Mirror the stream status into the tables' empty-state hint. The hit
    /// log is history and survives a disconnect untouched.
    fn set_connected(&mut self, connected: bool, cx: &mut Context<Self>) {
        for table in &self.tables {
            table.update(cx, |state, cx| {
                let delegate = state.delegate_mut();
                if delegate.connected == connected {
                    return;
                }
                delegate.connected = connected;
                cx.notify();
            });
        }
    }

    /// Switch to `name` and resync on the next render. Re-selecting the
    /// current preset is a no-op; the toolbar refresh button is the explicit
    /// way to refetch the pool.
    fn select_preset(&mut self, name: String, cx: &mut Context<Self>) {
        if self.preset.as_deref() == Some(name.as_str()) {
            return;
        }
        self.preset = Some(name);
        self.pending_resync = true;
        cx.notify();
    }

    /// Fetch the selected preset's Finviz export into the pool. Guards run
    /// before anything is marked loading; a fetch whose generation has been
    /// superseded (another fetch, a preset switch) discards its outcome.
    ///
    /// 本方法总在面板自身的 update 内被调(连接观察者/refresh/resync),
    /// 所以不能走 [`crate::data::DataRequest`]——它的 begin 阶段会对状态
    /// 实体做 `update`,在实体自己的 update 里再 update 自己即重入崩溃。
    /// 同步半段直接改字段(`&mut self` 本就在手),异步回写经 spawn,
    /// 天然在本轮 update 之外。
    fn fetch(&mut self, cx: &mut Context<Self>) {
        let Some(preset_name) = self.preset.clone() else {
            return;
        };
        let global = cx.global::<AppState>();
        let Some(preset) = global
            .settings
            .market_screener
            .presets
            .get(&preset_name)
            .cloned()
        else {
            return;
        };
        let Some(parameter) = global
            .settings
            .finviz
            .screener_parameter
            .get(&preset.finviz_preset)
            .cloned()
        else {
            message::warning(
                tr!(
                    "app.market_screener.referenced_missing",
                    name = preset.finviz_preset.clone()
                ),
                format!("finviz preset {} missing", preset.finviz_preset),
                cx,
            );
            return;
        };
        // Finviz token 已下沉服务端;唯一的前置条件是 server 在线。
        // 离线时静默跳过:启动自动加载等 Connected 后由订阅回调补抓。
        let Ok(server) = crate::data::server_client(cx) else {
            return;
        };

        self.fetch_generation += 1;
        let generation = self.fetch_generation;
        for table in &self.tables {
            table.update(cx, |state, cx| {
                state.delegate_mut().is_loading = true;
                cx.notify();
            });
        }
        cx.spawn(async move |this, cx| {
            let result = crate::data::api::screener::market_export(&server, &parameter).await;
            let _ = this.update(cx, |this, cx| {
                if this.fetch_generation != generation {
                    // Superseded while in flight: discard silently, the
                    // newer fetch owns the loading flags now.
                    return;
                }
                match result {
                    Ok(csv) => {
                        let mut tickers = extract_tickers(&csv);
                        // The pool cap is a settings knob (0 = unlimited),
                        // re-read per fetch so an edit applies on the next
                        // refresh or preset switch.
                        if let Some(limit) = cx
                            .global::<AppState>()
                            .settings
                            .market_screener
                            .pool_limit()
                            && tickers.len() > limit
                        {
                            tickers.truncate(limit);
                            message::info(tr!("app.market_screener.truncated", max = limit), cx);
                        }
                        this.pool = tickers;
                        this.apply_pool(cx);
                        this.sync_subscription(cx);
                    }
                    Err(err) => {
                        message::error(tr!("app.market_screener.failed"), err, cx);
                    }
                }
                for table in &this.tables {
                    table.update(cx, |state, cx| {
                        state.delegate_mut().is_loading = false;
                        cx.notify();
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Save `draft` (renaming when it used to live under another title) and
    /// select it. The settings observer diffs the snapshot and rebuilds or
    /// refetches as needed.
    fn upsert_preset(
        &mut self,
        previous: Option<&str>,
        draft: MarketPresetDraft,
        cx: &mut Context<Self>,
    ) {
        settings::update_settings(cx, |s| {
            let presets = &mut s.market_screener.presets;
            if let Some(previous) = previous
                && previous != draft.name
            {
                presets.remove(previous);
            }
            presets.insert(
                draft.name.clone(),
                MarketScreenerPreset {
                    exchanges: draft.exchanges,
                    min_size: draft.min_size,
                    finviz_preset: draft.finviz_preset,
                },
            );
        });
        self.preset = Some(draft.name);
        self.apply_pool(cx);
        cx.notify();
    }

    /// Delete `name`. Dropping it from the selection (with the fallback and
    /// the table rebuild) rides the settings observer.
    fn delete_preset(&mut self, name: &str, cx: &mut Context<Self>) {
        settings::update_settings(cx, |s| {
            s.market_screener.presets.remove(name);
        });
    }
}

impl gpui_kit::base::dock::Panel for MarketScreenerPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    /// Persist the selected preset per instance; `new` reads it back.
    fn dump(&self, _cx: &App) -> PanelState {
        let mut state = PanelState::new(Self::NAME);
        state.info = PanelInfo::Panel(serde_json::json!({ "preset": self.preset }));
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

impl EventEmitter<PanelEvent> for MarketScreenerPanel {}

impl gpui_kit::Focusable for MarketScreenerPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for MarketScreenerPanel {
    /// The selected preset as the tab label; the panel name until one
    /// exists.
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .preset
            .clone()
            .map(SharedString::from)
            .unwrap_or_else(|| tr!(Self::TITLE_KEY));
        panel_title(Self::ICON, label)
    }

    /// Title-bar button: refresh the pool from the selected preset's Finviz
    /// source. Preset management (add/delete) lives in the picker dropdown's
    /// footer, the same spot the stock screener uses.
    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        Some(vec![refresh_button::<Self>(
            "market-refresh",
            cx.weak_entity(),
        )])
    }

    /// The preset picker rides the tab title bar, next to the toolbar
    /// buttons. It is pinned to a fixed width: the combobox inside is
    /// `w_full`, and unbounded it would eat the whole tab row (the
    /// DatePicker trap). The dropdown's footer carries the shared management
    /// buttons: plus opens the blank preset form, pencil reopens it seeded
    /// with the selected preset, trash (armed only while a preset is
    /// selected) confirms the delete.
    fn title_suffix(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let weak = cx.weak_entity();

        Some(
            div()
                .flex_shrink_0()
                .w(px(200.))
                .child(self.picker.managed_element(
                    tr!("app.market_screener.pick_preset"),
                    {
                        let weak = weak.clone();
                        move |cx| weak.upgrade().and_then(|p| p.read(cx).preset.clone())
                    },
                    Some(PickerAction::new(
                        "market-add-preset",
                        AppIcon::Plus,
                        tr!("app.market_screener.add_preset"),
                        {
                            let weak = weak.clone();
                            move |window, cx, _| {
                                open_market_preset_dialog(window, cx, None, {
                                    let weak = weak.clone();
                                    move |draft, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            this.upsert_preset(None, draft, cx)
                                        });
                                    }
                                });
                            }
                        },
                    )),
                    Some(PickerAction::new(
                        "market-edit-preset",
                        AppIcon::Pencil,
                        tr!("app.market_screener.edit_preset"),
                        {
                            let weak = weak.clone();
                            move |window, cx, selected| {
                                let Some(name) = selected else {
                                    return;
                                };
                                let Some(preset) = cx
                                    .global::<AppState>()
                                    .settings
                                    .market_screener
                                    .presets
                                    .get(&name)
                                    .cloned()
                                else {
                                    return;
                                };
                                let initial = MarketPresetDraft {
                                    name: name.clone(),
                                    exchanges: preset.exchanges,
                                    min_size: preset.min_size,
                                    finviz_preset: preset.finviz_preset,
                                };
                                let previous = name;
                                open_market_preset_dialog(window, cx, Some(initial), {
                                    let weak = weak.clone();
                                    move |draft, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            this.upsert_preset(Some(&previous), draft, cx)
                                        });
                                    }
                                });
                            }
                        },
                    )),
                    Some(PickerAction::new(
                        "market-delete-preset",
                        AppIcon::Trash,
                        tr!("app.market_screener.delete_preset"),
                        {
                            let weak = weak.clone();
                            move |window, cx, selected| {
                                let Some(name) = selected else {
                                    return;
                                };
                                // Fresh clone inside the `Fn` body: the
                                // confirm callback takes its own `weak`,
                                // and moving the closure's capture out
                                // would break `Fn`.
                                let weak = weak.clone();
                                open_confirm_dialog(
                                    window,
                                    cx,
                                    tr!("app.market_screener.delete_preset"),
                                    tr!("app.market_screener.delete_confirm", name = name.clone()),
                                    move |cx| {
                                        let _ = weak
                                            .update(cx, |this, cx| this.delete_preset(&name, cx));
                                    },
                                );
                            }
                        },
                    )),
                )),
        )
    }
}

impl Render for MarketScreenerPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pending_resync {
            self.pending_resync = false;
            self.resync(window, cx);
        }

        let content: AnyElement = if self.tables.is_empty() {
            let hint = if cx
                .global::<AppState>()
                .settings
                .market_screener
                .presets
                .is_empty()
            {
                tr!("app.market_screener.no_preset")
            } else {
                tr!("app.market_screener.pick_preset")
            };
            div()
                .size_full()
                .child(datatable::empty_view(hint, cx))
                .into_any_element()
        } else {
            // Every table is a fixed-width column; the row scrolls
            // horizontally when the exchanges exceed the panel's width.
            let mut row = h_flex()
                .id("market-tables")
                .size_full()
                .overflow_x_scroll()
                .gap_2()
                .p_2()
                .items_start();
            for (index, table) in self.tables.iter().enumerate() {
                let code = self.exchanges.get(index).cloned().unwrap_or_default();
                row = row.child(
                    v_flex()
                        .w(px(160.))
                        .flex_shrink_0()
                        .h_full()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(SharedString::from(exchange_label(&code))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .child(datatable::table_render(table)),
                        ),
                );
            }
            row.into_any_element()
        };
        div().size_full().child(content)
    }
}
