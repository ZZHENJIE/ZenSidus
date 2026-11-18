//! The main window: title-bar menus, the primary dock layout and the status
//! bar. It is the app's anchor — always restored at startup, and closing it
//! asks whether to quit.

pub mod close_event;
pub mod status_bar;

use crate::{
    panels,
    utils::message::MessageLog,
    utils::state::{RelayHubEvent, AppState},
    widgets::menu::{self, OnActionsExt},
    windows::{AppWindow, DockWindow, WindowType, dump_dock},
};
use gpui_kit::{
    App, Context, Entity, IntoElement, ParentElement, Render, ScrollHandle, Styled as _,
    Subscription, Window,
    base::dock::{DockArea, DockLayout, DockPlacement},
    component::{dock::panel_handle, v_flex},
    px,
};

pub struct MainWindow {
    dock_area: Entity<DockArea>,
    _dock_subscription: Subscription,
    /// Scroll position of the status bar's message-history popover; kept
    /// here so it survives re-renders instead of resetting on every push.
    message_scroll: ScrollHandle,
}

impl AppWindow for MainWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        window_type: WindowType,
        cache: serde_json::Value,
    ) -> Self {
        let (dock_area, _dock_subscription) = DockWindow::new_dock(window_type, &cache, window, cx);
        // The main window never closes on its own: the platform close
        // request opens the quit dialog instead, which persists every open
        // window and exits. (`hook_should_close` is deliberately not used.)
        window.on_window_should_close(cx, close_event::on_close);

        // Status bar inputs: message pushes notify this global observer, and
        // connection status changes arrive from the hub's background thread.
        cx.observe_global::<MessageLog>(|_, cx| cx.notify())
            .detach();
        let hub = cx.global::<AppState>().relay_hub.clone();
        cx.subscribe(&hub, |_, _, _: &RelayHubEvent, cx| {
            cx.notify();
        })
        .detach();

        Self {
            dock_area,
            _dock_subscription,
            message_scroll: ScrollHandle::new(),
        }
    }

    fn window_type(&self) -> WindowType {
        WindowType::Main
    }

    fn default_window_type() -> WindowType {
        WindowType::Main
    }

    fn dump_cache(&mut self, cx: &mut App) -> serde_json::Value {
        dump_dock::<Self>(&self.dock_area, cx)
    }

    fn render_titlebar_left(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        menu::layout()
            .child(menu::app::menu())
            .child(menu::panels::menu())
            .child(menu::windows::menu())
    }

    fn render_content(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .child(self.dock_area.clone())
            .child(self.render_status_bar(cx))
    }
}

impl DockWindow for MainWindow {
    fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }

    /// Out-of-the-box layout: watchlist left, chart center, stock news right.
    fn build_default_layout(area: &mut DockArea, window: &mut Window, cx: &mut Context<DockArea>) {
        let watchlist = panels::fresh::<panels::WatchlistPanel>(window, cx);
        let chart = panels::fresh::<panels::ChartPanel>(window, cx);
        let stock_news = panels::fresh::<panels::StockNewsPanel>(window, cx);

        area.set_center(
            DockLayout::tabs().panel_view(panel_handle(chart), cx),
            window,
            cx,
        );
        area.set_dock(
            DockPlacement::Left,
            DockLayout::tabs().panel_view(panel_handle(watchlist), cx),
            window,
            cx,
        );
        area.set_dock_size(DockPlacement::Left, px(280.), window, cx);
        area.set_dock(
            DockPlacement::Right,
            DockLayout::tabs().panel_view(panel_handle(stock_news), cx),
            window,
            cx,
        );
        area.set_dock_size(DockPlacement::Right, px(320.), window, cx);
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .on_actions(menu::app::actions, cx)
            .on_actions(menu::panels::actions, cx)
            .on_actions(menu::windows::actions, cx)
            .child(self.render_window(window, cx))
    }
}
