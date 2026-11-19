//! Market news panel: the market-wide news feed. History comes from one RPC
//! page (the push channel only carries new articles); from then on the
//! server's news broadcast inserts live articles as they arrive, merged into
//! the RPC rows by id. The pump runs for the panel's whole lifetime — news
//! is low-frequency, so consuming it costs nothing and the table stays
//! current without a catch-up fetch on re-activation. The server pushes
//! news to every connected client (no per-symbol gating), so the panel only
//! waits for the hub's `Connected` state, no subscription frame of its own.

use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, IntoElement, ParentElement as _, Render,
    Styled as _, Task, WeakEntity, Window,
    base::dock::PanelEvent,
    component::{button::Button, table::TableState},
    div,
};
use protocol::{RpcRequestPayload, RpcResponsePayload};
use tokio::sync::broadcast;

use crate::panels::{Panel, refresh_button};
use crate::tr;
use crate::utils::state::AppState;
use crate::utils::{AppIcon, message};
use crate::widgets::datatable::{self, Loadable as _, MarketNewsDelegate};

pub struct MarketNewsPanel {
    focus_handle: FocusHandle,
    table: Entity<TableState<MarketNewsDelegate>>,
    /// Live message pump over the server's news broadcast. It runs for the
    /// panel's whole lifetime; `None` until the hub reports `Connected`
    /// (or while the server is down after a permanent failure).
    pump: Option<Task<()>>,
}

impl MarketNewsPanel {
    /// Load one page of recent market news over the server and merge it into
    /// the table. `load_rows` is deliberately not used here: it replaces
    /// rows wholesale, which would drop articles that arrived on the push
    /// channel while the request was in flight — the merge keeps them, with
    /// the same generation guard against superseded responses.
    fn backfill(table: WeakEntity<TableState<MarketNewsDelegate>>, cx: &mut App) {
        let Some(table) = table.upgrade() else {
            return;
        };
        let Ok(server) = AppState::server(cx) else {
            return;
        };
        let generation = table.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            *delegate.loading_mut() = true;
            *delegate.fetch_generation_mut() += 1;
            cx.notify();
            *delegate.fetch_generation_mut()
        });

        cx.spawn(async move |cx| {
            let result = async {
                match server
                    .rpc_checked(RpcRequestPayload::News {
                        symbols: vec![],
                        limit: 50,
                    })
                    .await?
                {
                    RpcResponsePayload::News(items) => Ok(items),
                    other => Err(anyhow::anyhow!("unexpected rpc response: {other:?}")),
                }
            }
            .await;
            table.update(cx, |state, cx| {
                let delegate = state.delegate_mut();
                if *delegate.fetch_generation_mut() != generation {
                    // A newer fetch started meanwhile; this outcome is stale.
                    return;
                }
                *delegate.loading_mut() = false;
                match result {
                    Ok(articles) => {
                        delegate.merge_articles(articles);
                    }
                    Err(err) => {
                        message::error(tr!("app.market_news.failed"), err, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Attach the broadcast-receiver pump (once): one receiver of the
    /// server's news broadcast, inserting every article into the table.
    /// Silent no-op while the server is down; the hub-status observer calls
    /// again on `Connected`.
    fn try_attach(&mut self, cx: &mut Context<Self>) {
        if self.pump.is_some() {
            return;
        }
        let Ok(server) = AppState::server(cx) else {
            return;
        };
        let mut rx = server.news_messages();
        let weak = self.table.downgrade();
        self.pump = Some(cx.spawn(async move |_this, cx| {
            loop {
                match rx.recv().await {
                    Ok(news) => {
                        // An Err here means the panel is gone (the delegate
                        // entity was dropped): end the pump with it.
                        if weak
                            .update(cx, |state, cx| {
                                if state
                                    .delegate_mut()
                                    .merge_articles(vec![protocol::NewsItem::from(news)])
                                {
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    // A brief data gap only momentarily stales the feed:
                    // a news panel stays quiet in the message center.
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(
                            "{}",
                            tr!("app.log.missed_stream_messages", count = skipped)
                        );
                    }
                    Err(_) => break,
                }
            }
        }));
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
            pump: None,
        };
        // Follow settings changes (e.g. a language switch) to retranslate
        // the column headers, and attach once the hub reports `Connected`.
        let hub = cx.global::<AppState>().hub.clone();
        cx.observe(&hub, |this, _, cx| this.try_attach(cx)).detach();
        cx.observe_global_in::<AppState>(window, |_, _, cx| cx.notify())
            .detach();
        // Attach right away when the server is already up (panel re-created
        // after a hub connect, for instance).
        this.try_attach(cx);
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
            self.try_attach(cx);
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
