//! Stock news panel: one page of Alpaca news for a followed symbol,
//! persisted per panel instance. The symbol is picked through the search dialog.

use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, IntoElement, ParentElement as _, Render,
    Styled, WeakEntity, Window,
    base::dock::{PanelEvent, PanelInfo, PanelState},
    component::{button::Button, table::TableState},
    div,
};

use crate::panels::{Panel, panel_title, refresh_button};
use crate::tr;
use crate::utils::{AppIcon, state::AppState};
use crate::widgets::datatable::{self, StockNewsDelegate};
use crate::widgets::{dialog::open_search_symbol_dialog, icon_button};

pub struct StockNewsPanel {
    focus_handle: FocusHandle,
    table: Entity<TableState<StockNewsDelegate>>,
}

impl StockNewsPanel {
    fn symbol(&self, cx: &App) -> Option<String> {
        self.table.read(cx).delegate().symbol.clone()
    }

    /// Fetch one page of news for the followed symbol. Offline the fetch is
    /// skipped silently (the connection lands later via `reload_on_connect`).
    fn fetch(table: WeakEntity<TableState<StockNewsDelegate>>, cx: &mut App) {
        let Some(symbol) = table
            .upgrade()
            .and_then(|table| table.read(cx).delegate().symbol.clone())
        else {
            return;
        };

        datatable::load_rows(
            table,
            tr!("app.stock_news.failed"),
            move |server| async move {
                crate::data::api::news::for_symbols(&server, vec![symbol], 50).await
            },
            cx,
        );
    }
}

impl Panel for StockNewsPanel {
    type DataEntity = TableState<StockNewsDelegate>;
    const NAME: &'static str = "StockNewsPanel";
    const ICON: AppIcon = AppIcon::News;
    const TITLE_KEY: &'static str = "app.panels.stock_news";

    fn new(window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self {
        let persisted_symbol = info
            .get("symbol")
            .and_then(|value| value.as_str())
            .map(String::from);
        let this = Self {
            focus_handle: cx.focus_handle(),
            table: datatable::new_table_state(cx, window, |_| {
                StockNewsDelegate::new(persisted_symbol.clone(), vec![])
            }),
        };
        // Follow settings changes (e.g. a language switch) to retranslate
        // the column headers. The data itself is not settings-dependent, so
        // no refetch.
        cx.observe_global_in::<AppState>(window, |_, _, cx| cx.notify())
            .detach();
        // Auto-load once when a symbol was persisted.
        if this.symbol(cx).is_some() {
            Self::fetch(this.table.downgrade(), cx);
        }
        // Offline at startup: the fetch above was skipped silently; load
        // once the connection lands (refresh itself re-checks the symbol).
        crate::panels::reload_on_connect::<Self>(this.table.downgrade(), cx);
        this
    }

    fn refresh(entity: WeakEntity<Self::DataEntity>, cx: &mut App) {
        Self::fetch(entity, cx)
    }
}

impl gpui_kit::base::dock::Panel for StockNewsPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    /// Persist the followed symbol per instance; `new` reads it back and
    /// auto-loads.
    fn dump(&self, cx: &App) -> PanelState {
        let mut state = PanelState::new(Self::NAME);
        state.info = PanelInfo::Panel(serde_json::json!({ "symbol": self.symbol(cx) }));
        state
    }
}

impl EventEmitter<PanelEvent> for StockNewsPanel {}

impl gpui_kit::Focusable for StockNewsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for StockNewsPanel {
    /// The followed symbol as the tab label; the panel name until one is set.
    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .symbol(cx)
            .map(gpui_kit::SharedString::from)
            .unwrap_or_else(|| tr!(Self::TITLE_KEY));
        panel_title(Self::ICON, label)
    }

    /// Title-bar buttons: pick a symbol through the search dialog; refetch.
    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        let weak = self.table.downgrade();
        Some(vec![
            icon_button(
                "news-search",
                AppIcon::Search,
                tr!("app.stock_news.search_tooltip"),
                false,
                {
                    let weak = weak.clone();
                    move |_, window, cx| {
                        open_search_symbol_dialog(window, cx, {
                            let weak = weak.clone();
                            move |item, cx| {
                                let _ = weak.update(cx, |state, cx| {
                                    state.delegate_mut().set_symbol(Some(item.symbol.clone()));
                                    cx.notify();
                                });
                                Self::fetch(weak.clone(), cx);
                            }
                        });
                    }
                },
            ),
            refresh_button::<Self>("news-refresh", weak),
        ])
    }
}

impl Render for StockNewsPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(datatable::table_render(&self.table))
    }
}
