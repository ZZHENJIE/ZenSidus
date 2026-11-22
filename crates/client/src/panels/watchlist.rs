//! Watchlist panel: one symbol group per panel instance with live trades
//! from the shared Alpaca stream. The data (selected group, symbols, quotes)
//! lives in the delegate — `refresh` reloads it from the shared settings —
//! while this panel owns the UI shell and the stream subscription.
//! Overlapping symbols across panels share one wire subscription through
//! the hub.

use std::collections::BTreeSet;

use anysdk::alpaca_stream::StockMessage;
use chrono::{DateTime, Local, Utc};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EntityId, EventEmitter, FocusHandle, IntoElement,
    ParentElement as _, Render, SharedString, Styled, Task, WeakEntity, Window,
    assets::IconName,
    base::dock::{PanelEvent, PanelInfo, PanelState},
    component::{
        WindowExt,
        combobox::ComboboxEvent,
        input::{Input, InputState},
        searchable_list::SearchableVec,
        table::TableState,
    },
    div, px,
};

use crate::panels::Panel;
use crate::tr;
use crate::utils::state::stream_dispatch::{SharedSpec, StreamSpec, spawn_filter_stage};
use crate::utils::state::{HubEvent, ServerStatus, AppState, SymbolWish};
use crate::utils::{AppIcon, settings};
use crate::widgets::datatable::{self, WatchlistDelegate};
use crate::widgets::dialog::{dialog_footer, open_confirm_dialog, open_search_symbol_dialog};
use crate::widgets::icon_button;
use crate::widgets::picker::{PickerAction, StringPicker};

pub struct WatchlistPanel {
    focus_handle: FocusHandle,
    picker: StringPicker,
    table: Entity<TableState<WatchlistDelegate>>,
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

impl Panel for WatchlistPanel {
    type DataEntity = TableState<WatchlistDelegate>;
    const NAME: &'static str = "WatchlistPanel";
    const ICON: AppIcon = AppIcon::Watchlist;
    const TITLE_KEY: &'static str = "app.panels.watchlist";

    fn new(window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self {
        let focus_handle = cx.focus_handle();
        let persisted_group = info
            .get("group")
            .and_then(|value| value.as_str())
            .map(String::from);

        let delegate = WatchlistDelegate::new(persisted_group, cx);
        let picker = StringPicker::new(group_names(cx), delegate.group.clone(), window, cx);
        let table = datatable::new_table_state(cx, window, |_| delegate);

        let hub = cx.global::<AppState>().hub.clone();

        // Connection status drives the message pump: connected attaches a
        // fresh broadcast receiver, any other status cancels it.
        cx.subscribe(&hub, |this, hub, _: &HubEvent, cx| {
            match hub.read(cx).status() {
                ServerStatus::Connected => this.refresh_pump(cx),
                _ => this.pump = None,
            }
        })
        .detach();

        // Picking a group in the picker switches the panel to it.
        cx.subscribe(
            picker.state(),
            |this, _, event: &ComboboxEvent<SearchableVec<String>>, cx| {
                if let ComboboxEvent::Change(values) = event
                    && let Some(name) = values.last()
                {
                    this.select_group(name.clone(), cx);
                }
            },
        )
        .detach();

        // Group lists are shared settings; follow edits made by other panels.
        // `refresh` lets the delegate settle its group first (a deleted one
        // falls back to the first remaining); the picker then mirrors it.
        cx.observe_global_in::<AppState>(window, |this, window, cx| {
            Self::refresh(this.table.downgrade(), cx);
            let current = this.group(cx);
            this.picker.set_items(group_names(cx), window, cx);
            this.picker.set_selected(current, window, cx);
            this.sync_subscription(cx);
            cx.notify();
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
            focus_handle,
            picker,
            table,
            active: true,
            subscriber,
            spec: SharedSpec::default(),
            pump: None,
        };
        this.sync_subscription(cx);
        this.refresh_pump(cx);
        this
    }

    /// Reload the delegate from the shared settings (the group persisted in
    /// `info` was already applied by `new`).
    fn refresh(entity: WeakEntity<Self::DataEntity>, cx: &mut App) {
        let _ = entity.update(cx, |state, cx| {
            state.delegate_mut().reload_from_settings(cx);
            cx.notify();
        });
    }
}

/// Group names in settings order (a `BTreeMap`, so "first" is stable).
fn group_names(cx: &App) -> Vec<String> {
    cx.global::<AppState>()
        .settings
        .watchlists
        .keys()
        .cloned()
        .collect()
}

impl WatchlistPanel {
    /// The panel's currently selected group (lives in the delegate).
    pub(crate) fn group(&self, cx: &App) -> Option<String> {
        self.table.read(cx).delegate().group.clone()
    }

    /// Publish this panel's wanted symbols to the hub. While the tab is not
    /// the active one the set is empty, unsubscribing its share of the union.
    fn sync_subscription(&mut self, cx: &mut Context<Self>) {
        let wanted: BTreeSet<String> = if self.active {
            self.group(cx)
                .and_then(|name| cx.global::<AppState>().settings.watchlists.get(&name))
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect()
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
        let hub = cx.global::<AppState>().hub.clone();
        hub.update(cx, |hub, _| {
            hub.set_symbols(
                self.subscriber,
                SymbolWish {
                    symbols: wanted,
                    trades: true,
                    bars: false,
                    quotes: false,
                },
            )
        });
    }

    /// (Re)attach the broadcast-receiver pump. Each pump owns one receiver of
    /// the hub's stream; the whole channel is broadcast to every receiver,
    /// so the drain batches whatever is queued — one entity update per batch
    /// instead of one per message keeps the main thread cheap even while
    /// other panels' high-rate subscriptions fill the channel.
    fn refresh_pump(&mut self, cx: &mut Context<Self>) {
        let hub = cx.global::<AppState>().hub.clone();
        let clock = hub.read(cx).clock();
        let Some(rx) = hub.read(cx).stock_messages() else {
            return;
        };
        let spec = self.spec.clone();
        let (tx, mut rx_out) = tokio::sync::mpsc::channel(8);
        cx.background_executor()
            .spawn(spawn_filter_stage("watchlist", rx, spec, tx, clock))
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

    /// Apply a whole drained batch (one entity update per pump cycle).
    fn apply_batch(&mut self, batch: Vec<StockMessage>, cx: &mut Context<Self>) {
        for message in batch {
            self.apply_message(message, cx);
        }
    }

    fn apply_message(&mut self, message: StockMessage, cx: &mut Context<Self>) {
        let (symbol, patch) = match message {
            StockMessage::Trade(trade) => (
                trade.symbol,
                Patch::Last(trade.price, format_time(trade.time)),
            ),
            StockMessage::DailyBar(bar) => (bar.symbol, Patch::Open(bar.open)),
            _ => return,
        };
        self.table.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            let applied = match patch {
                Patch::Last(price, time) => delegate.apply_trade(&symbol, price, time),
                Patch::Open(open) => delegate.apply_open(&symbol, open),
            };
            if applied {
                cx.notify();
            }
        });
    }

    fn select_group(&mut self, name: String, cx: &mut Context<Self>) {
        self.table.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            if delegate.group.as_deref() == Some(name.as_str()) {
                return;
            }
            delegate.group = Some(name);
            delegate.reload_from_settings(cx);
            cx.notify();
        });
        self.sync_subscription(cx);
        cx.notify();
    }

    fn add_group(&mut self, name: String, cx: &mut Context<Self>) {
        settings::update_settings(cx, |s| {
            s.watchlists.entry(name.clone()).or_default();
        });
        self.select_group(name, cx);
    }

    /// Delete `name`. Dropping its symbols and selecting the next group is
    /// handled by the settings observer's refresh.
    fn delete_group(&mut self, name: &str, cx: &mut Context<Self>) {
        settings::update_settings(cx, |s| {
            s.watchlists.remove(name);
        });
        self.sync_subscription(cx);
    }

    fn add_symbol(&mut self, symbol: String, cx: &mut Context<Self>) {
        let Some(group) = self.group(cx) else {
            return;
        };
        settings::update_settings(cx, |s| {
            if let Some(symbols) = s.watchlists.get_mut(&group)
                && !symbols.contains(&symbol)
            {
                symbols.push(symbol);
            }
        });
        // The settings observer reloads the table (keeping quote history) and
        // the picker; resubscription follows there too.
        Self::refresh(self.table.downgrade(), cx);
        self.sync_subscription(cx);
    }
}

enum Patch {
    Last(f64, SharedString),
    Open(f64),
}

fn format_time(time: DateTime<Utc>) -> SharedString {
    time.with_timezone(&Local)
        .format("%H:%M:%S")
        .to_string()
        .into()
}

impl gpui_kit::base::dock::Panel for WatchlistPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    /// Persist the selected group per instance; `new` reads it back.
    fn dump(&self, cx: &App) -> PanelState {
        let mut state = PanelState::new(Self::NAME);
        state.info = PanelInfo::Panel(serde_json::json!({ "group": self.group(cx) }));
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

impl EventEmitter<PanelEvent> for WatchlistPanel {}

impl gpui_kit::Focusable for WatchlistPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for WatchlistPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Self::default_title()
    }

    /// Title-bar button: search a symbol and add it to the selected group.
    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Vec<gpui_kit::component::button::Button>> {
        let has_group = self.group(cx).is_some();
        let weak = cx.weak_entity();
        Some(vec![icon_button(
            "watchlist-add-symbol",
            AppIcon::Search,
            tr!("app.symbol.search_title"),
            !has_group,
            move |_, window, cx| {
                open_search_symbol_dialog(window, cx, {
                    let weak = weak.clone();
                    move |item, cx| {
                        let _ =
                            weak.update(cx, |this, cx| this.add_symbol(item.symbol.clone(), cx));
                    }
                });
            },
        )])
    }

    /// The group picker rides the tab title bar, next to the toolbar
    /// buttons. It is pinned to a fixed width: the combobox inside is
    /// `w_full`, and unbounded it would eat the whole tab row and push the
    /// toolbar out of view (the DatePicker trap). Group management
    /// (new/delete) lives in the shared picker footer.
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
                    tr!("app.watchlist.pick_group"),
                    {
                        let weak = weak.clone();
                        move |cx| weak.upgrade().and_then(|p| p.read(cx).group(cx))
                    },
                    Some(PickerAction::new(
                        "watchlist-new-group",
                        IconName::Plus,
                        tr!("app.watchlist.new_group"),
                        {
                            let weak = weak.clone();
                            move |window, cx, _| {
                                open_add_group_dialog(&weak, window, cx);
                            }
                        },
                    )),
                    None,
                    Some(PickerAction::new(
                        "watchlist-delete-group",
                        IconName::Trash,
                        tr!("app.watchlist.delete_group"),
                        {
                            let weak = weak.clone();
                            move |window, cx, selected| {
                                let Some(name) = selected else {
                                    return;
                                };
                                open_delete_group_dialog(&weak, name, window, cx);
                            }
                        },
                    )),
                )),
        )
    }
}

impl Render for WatchlistPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(datatable::table_render(&self.table))
    }
}

/// Prompt for a new group name; confirming creates and selects it.
fn open_add_group_dialog(weak: &WeakEntity<WatchlistPanel>, window: &mut Window, cx: &mut App) {
    let weak = weak.clone();
    let name_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(tr!("app.watchlist.new_group_title")));

    window.open_dialog(cx, move |dialog, _window, _cx| {
        let weak = weak.clone();
        let confirm = {
            let name_input = name_input.clone();
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                let name = name_input.read(cx).value().trim().to_string();
                if name.is_empty() {
                    return;
                }
                let _ = weak.update(cx, |this, cx| this.add_group(name, cx));
                window.close_dialog(cx);
            }
        };

        dialog
            .title(tr!("app.watchlist.new_group_title"))
            .child(Input::new(&name_input))
            .footer(dialog_footer("watchlist-add", confirm))
    });
}

/// Confirm deleting `name`, showing how many symbols go with it.
fn open_delete_group_dialog(
    weak: &WeakEntity<WatchlistPanel>,
    name: String,
    window: &mut Window,
    cx: &mut App,
) {
    let weak = weak.clone();
    let count = cx
        .global::<AppState>()
        .settings
        .watchlists
        .get(&name)
        .map(Vec::len)
        .unwrap_or(0);

    open_confirm_dialog(
        window,
        cx,
        tr!("app.watchlist.delete_group"),
        tr!(
            "app.watchlist.delete_group_confirm",
            name = name.clone(),
            count = count
        ),
        move |cx| {
            let _ = weak.update(cx, |this, cx| this.delete_group(&name, cx));
        },
    );
}
