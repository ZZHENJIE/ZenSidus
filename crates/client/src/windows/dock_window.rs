//! Shared dock-area plumbing for windows whose content is a panel layout
//! (currently [`crate::windows::MainWindow`] and
//! [`crate::windows::CustomWindow`]).
//!
//! Layout persistence goes into the window's `WindowState.cache` (inside
//! `settings.json`) — there is no separate layout file. Every
//! [`DockEvent::LayoutChanged`] re-dumps the area, so the persisted layout is
//! always current, even across crashes.

use gpui_kit::{
    App, Context, Entity, Subscription, Window,
    base::dock::{DockArea, DockAreaState, DockEvent},
    component::dock::{DockSkin, PanelStyle},
};

use crate::windows::{AppWindow, WindowType, update_window_state};

/// Schema version of the persisted dock layout. Bump whenever a window's
/// default layout or panel structure changes, so stale caches fall back to
/// the fresh default instead of restoring an outdated layout.
const DOCK_VERSION: usize = 5;

pub trait DockWindow: AppWindow {
    /// The window's dock area entity.
    fn dock_area(&self) -> &Entity<DockArea>;

    /// Build the out-of-the-box layout for a fresh window (no valid cache).
    fn build_default_layout(area: &mut DockArea, window: &mut Window, cx: &mut Context<DockArea>);

    /// Whether the window shows only the center dock. Left/right/bottom
    /// docks are stripped from both loaded and saved state.
    fn center_only() -> bool {
        false
    }

    /// Persisted-layout version this window understands; see [`DOCK_VERSION`].
    fn dock_version() -> Option<usize> {
        Some(DOCK_VERSION)
    }

    /// Create the dock area: restore the persisted layout from `cache`,
    /// falling back to [`Self::build_default_layout`]. Subscribes to layout
    /// changes so every edit is persisted immediately.
    fn new_dock(
        window_type: WindowType,
        cache: &serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<DockArea>, Subscription)
    where
        Self: Sized,
    {
        let (area, skin) =
            DockSkin::dock_area(window_type.dock_name(), Self::dock_version(), window, cx);
        skin.set_panel_style(PanelStyle::TabBar, cx);

        // Only same-version caches are restored; anything older (or corrupt)
        // falls back to the current default layout.
        let mut restored = false;
        if let Ok(mut state) = serde_json::from_value::<DockAreaState>(cache.clone())
            && state.version == Self::dock_version()
        {
            if Self::center_only() {
                strip_side_docks(&mut state);
            }
            restored = area
                .update(cx, |area, cx| area.load(state, window, cx))
                .is_ok();
        }
        if !restored {
            area.update(cx, |area, cx| Self::build_default_layout(area, window, cx));
        }

        let subscription = cx.subscribe(&area, |this, _, event: &DockEvent, cx| {
            if matches!(event, DockEvent::LayoutChanged) {
                this.persist_layout(cx);
            }
        });
        (area, subscription)
    }

    /// Snapshot the current layout into the window's persisted cache.
    fn persist_layout(&mut self, cx: &mut Context<Self>)
    where
        Self: Sized,
    {
        let cache = dump_dock::<Self>(self.dock_area(), cx);
        let window_type = self.window_type();
        update_window_state(cx, window_type, |state| state.cache = cache);
    }

    /// Reset to the default layout and persist it.
    fn reset_layout(&mut self, window: &mut Window, cx: &mut Context<Self>)
    where
        Self: Sized,
    {
        self.dock_area()
            .update(cx, |area, cx| Self::build_default_layout(area, window, cx));
        self.persist_layout(cx);
    }
}

/// Dump a dock area into the JSON stored in `WindowState.cache`.
pub fn dump_dock<W: DockWindow>(area: &Entity<DockArea>, cx: &App) -> serde_json::Value {
    let mut state = area.read(cx).dump(cx);
    if W::center_only() {
        strip_side_docks(&mut state);
    }
    serde_json::to_value(state).unwrap_or(serde_json::Value::Null)
}

/// Drop the left/right/bottom docks from a layout, leaving only the center.
fn strip_side_docks(state: &mut DockAreaState) {
    state.left_dock = None;
    state.right_dock = None;
    state.bottom_dock = None;
}
