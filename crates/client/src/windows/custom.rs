use gpui_kit::{
    App, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window,
    base::{
        dock::{DockArea, DockLayout},
        v_flex,
    },
    component::dock::panel_handle,
};

use crate::{
    panels,
    widgets::menu::{self, OnActionsExt},
    windows::{AppWindow, DockWindow, WindowType, dump_dock, hook_should_close},
};

/// A user-created window holding its own panel layout. It has no left/right
/// docks — only the center pane.
///
/// Instances are numbered; [`CustomWindow::open`] picks the smallest id that
/// is not currently open, so closing one and opening another brings back the
/// closed instance's geometry and layout (persisted in its
/// `WindowState.cache`).
pub struct CustomWindow {
    id: u32,
    dock_area: Entity<DockArea>,
    _dock_subscription: Subscription,
}

impl AppWindow for CustomWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        window_type: WindowType,
        cache: serde_json::Value,
    ) -> Self {
        let (dock_area, _dock_subscription) = DockWindow::new_dock(window_type, &cache, window, cx);
        hook_should_close(window, cx);
        Self {
            id: window_type.instance_id().unwrap_or(0),
            dock_area,
            _dock_subscription,
        }
    }

    fn open(cx: &mut App) {
        let id = super::allocate_instance_id(cx, WindowType::Custom(0));
        Self::open_instance(WindowType::Custom(id), cx);
    }

    fn window_type(&self) -> WindowType {
        WindowType::Custom(self.id)
    }

    fn default_window_type() -> WindowType {
        WindowType::Custom(0)
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
            .child(super::titlebar_title(self.window_type().title()))
            .child(menu::panels::menu())
    }

    fn render_content(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.dock_area.clone()
    }
}

impl DockWindow for CustomWindow {
    fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }

    /// v3: the default layout now seeds a Chart panel in the center
    /// (previously empty). Older custom-window caches fall back to this.
    fn dock_version() -> Option<usize> {
        Some(3)
    }

    /// New custom windows start with a single Chart panel in the center.
    fn build_default_layout(area: &mut DockArea, window: &mut Window, cx: &mut Context<DockArea>) {
        let chart = panels::fresh::<panels::ChartPanel>(window, cx);
        area.set_center(
            DockLayout::tabs().panel_view(panel_handle(chart), cx),
            window,
            cx,
        );
    }

    fn center_only() -> bool {
        true
    }
}

impl Render for CustomWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .on_actions(menu::panels::actions, cx)
            .child(self.render_window(window, cx))
    }
}
