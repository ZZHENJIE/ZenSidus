//! Market news panel: the market-wide news feed, **push-only** — the panel
//! subscribes the server's NEWS channel on attach and every pushed article
//! is merged into the table by id (the pump lives in the data layer).
//! 无拉取:表格从订阅时刻起累积,历史页不在客户端拉取。

use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, IntoElement, ParentElement as _, Render,
    Styled as _, WeakEntity, Window, base::dock::PanelEvent, component::table::TableState, div,
};

use crate::data::api::NewsItem;
use crate::data::stream::{NewsConsumer, NewsPump};
use crate::panels::Panel;
use crate::utils::AppIcon;
use crate::utils::state::AppState;
use crate::widgets::datatable::{self, MarketNewsDelegate};

pub struct MarketNewsPanel {
    focus_handle: FocusHandle,
    table: Entity<TableState<MarketNewsDelegate>>,
    /// The market-wide news push, merged into the table live.
    news: NewsPump,
}

impl NewsConsumer for MarketNewsPanel {
    fn news_pump(&mut self) -> &mut NewsPump {
        &mut self.news
    }

    /// Insert one pushed article into the table (merged by id).
    fn on_news(&mut self, item: &NewsItem, cx: &mut Context<Self>) {
        self.table.update(cx, |state, cx| {
            if state.delegate_mut().merge_articles(vec![item.clone()]) {
                cx.notify();
            }
        });
    }
}

impl Panel for MarketNewsPanel {
    type DataEntity = TableState<MarketNewsDelegate>;
    const NAME: &'static str = "MarketNewsPanel";
    const ICON: AppIcon = AppIcon::News;
    const TITLE_KEY: &'static str = "app.panels.market_news";

    fn new(window: &mut Window, cx: &mut Context<Self>, _info: &serde_json::Value) -> Self {
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            table: datatable::new_table_state(cx, window, |_| MarketNewsDelegate::new(vec![])),
            news: NewsPump::new(cx),
        };
        // Follow settings changes (e.g. a language switch) to retranslate
        // the column headers; the push pump follows the hub status.
        cx.observe_global_in::<AppState>(window, |_, _, cx| cx.notify())
            .detach();
        // Attach right away when the server is already up (panel re-created
        // after a hub connect, for instance).
        this.news.attach(cx);
        this
    }

    /// 纯推送,无拉取(refresh 是 Panel trait 的必填钩子,留空)。
    fn refresh(_entity: WeakEntity<Self::DataEntity>, _cx: &mut App) {}
}

impl gpui_kit::base::dock::Panel for MarketNewsPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn set_active(&mut self, active: bool, _window: &mut Window, cx: &mut Context<Self>) {
        // Nothing to do on hide: the pump keeps consuming while the tab is
        // hidden, so the table never goes stale. On re-activation, attach
        // if the pump is still missing (server came up meanwhile).
        if active {
            self.news.attach(cx);
        }
    }
}

impl EventEmitter<PanelEvent> for MarketNewsPanel {}

impl gpui_kit::Focusable for MarketNewsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for MarketNewsPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Self::default_title()
    }

    // 无工具栏:推送即数据源,没有可手动触发的拉取。
}

impl Render for MarketNewsPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(datatable::table_render(&self.table))
    }
}
