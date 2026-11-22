//! Market news panel: the market-wide news feed. History comes from one RPC
//! page (the push channel only carries new articles); from then on the
//! server's news broadcast inserts live articles as they arrive, merged into
//! the RPC rows by id — the push lives in the data layer's [`NewsPump`].

use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, IntoElement, ParentElement as _, Render,
    Styled as _, WeakEntity, Window,
    base::dock::PanelEvent,
    component::{button::Button, table::TableState},
    div,
};

use crate::data::api::NewsItem;
use crate::data::stream::{NewsConsumer, NewsPump};
use crate::panels::{Panel, refresh_button};
use crate::tr;
use crate::utils::AppIcon;
use crate::utils::state::{AppState, ServerStatus};
use crate::widgets::datatable::{self, Loadable as _, MarketNewsDelegate};

pub struct MarketNewsPanel {
    focus_handle: FocusHandle,
    table: Entity<TableState<MarketNewsDelegate>>,
    /// The market-wide news push, merged into the table live.
    news: NewsPump,
}

impl MarketNewsPanel {
    /// Load one page of recent market news and merge it into the table.
    /// `load_rows` is deliberately not used here: it replaces rows
    /// wholesale, which would drop articles that arrived on the push
    /// channel while the request was in flight — the merge keeps them, with
    /// the same generation guard against superseded responses.
    fn backfill(table: WeakEntity<TableState<MarketNewsDelegate>>, cx: &mut App) {
        crate::data::DataRequest::new(table, tr!("app.market_news.failed")).run(
            cx,
            |state, cx| {
                let delegate = state.delegate_mut();
                *delegate.loading_mut() = true;
                *delegate.fetch_generation_mut() += 1;
                cx.notify();
                Some(*delegate.fetch_generation_mut())
            },
            |state, generation| *state.delegate_mut().fetch_generation_mut() == generation,
            |server| async move { crate::data::api::news::market(&server, 50).await },
            |state, _cx, _generation, articles| {
                let delegate = state.delegate_mut();
                *delegate.loading_mut() = false;
                delegate.merge_articles(articles);
            },
            // 失败不清行:推送已到的文章保留。
            |state, _cx, _generation, _err| {
                *state.delegate_mut().loading_mut() = false;
            },
        );
    }
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

    fn on_news_status(
        &mut self,
        status: &ServerStatus,
        newly_attached: bool,
        cx: &mut Context<Self>,
    ) {
        // First attach after a fresh connect: the startup backfill was
        // skipped silently while offline, so pull the recent page now.
        if newly_attached && matches!(status, ServerStatus::Connected) {
            Self::backfill(self.table.downgrade(), cx);
        }
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

    fn refresh(entity: WeakEntity<Self::DataEntity>, cx: &mut App) {
        Self::backfill(entity, cx)
    }
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

    /// Title-bar button: pull the latest page over RPC (the push channel
    /// keeps the table live in between).
    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        let weak = self.table.downgrade();
        Some(vec![refresh_button::<Self>("market-news-refresh", weak)])
    }
}

impl Render for MarketNewsPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(datatable::table_render(&self.table))
    }
}
