//! The [`AppWindow`] contract every window implements, plus the shared
//! title-bar shell (custom-drawn: menu area, drag region, window controls).

use crate::utils::{AppIcon, AppState};
use crate::windows::{
    WindowState, WindowType, default_bounds, find_open_window, mark_window_closed,
    register_open_window, restore_display, update_window_state, window_options,
};
use gpui_kit::{
    App, AppContext, Bounds, Context, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement, Pixels, Point, Render, SharedString, Size, StatefulInteractiveElement as _,
    Styled, WeakEntity, Window, WindowControlArea,
    component::{ActiveTheme as _, Icon, Sizable as _, TITLE_BAR_HEIGHT, h_flex, v_flex},
    div,
    prelude::FluentBuilder as _,
    px,
};

#[cfg(target_os = "macos")]
const TITLE_BAR_LEFT_PADDING: Pixels = px(80.0);
#[cfg(not(target_os = "macos"))]
const TITLE_BAR_LEFT_PADDING: Pixels = px(12.0);

/// Bounds of a `width`×`height` window centered on the primary display.
pub(crate) fn centered_bounds_size(cx: &App, width: f32, height: f32) -> Bounds<Pixels> {
    let size = Size {
        width: px(width),
        height: px(height),
    };
    let bounds = cx
        .primary_display()
        .expect("no primary display found")
        .bounds();
    Bounds {
        origin: Point {
            x: bounds.origin.x + (bounds.size.width - size.width) / 2.,
            y: bounds.origin.y + (bounds.size.height - size.height) / 2.,
        },
        size,
    }
}

/// Plain window title text for windows without a menu bar.
pub fn titlebar_title(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

/// The application window contract.
///
/// Everything window-management related lives in provided methods:
/// opening (with singleton focus + geometry/cache restore), closing
/// (persistence of geometry, display and cache), and the shared titlebar
/// shell. Concrete windows only implement content.
pub trait AppWindow: Render + Sized {
    /// Build the window from its identity and persisted cache payload.
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        window_type: WindowType,
        cache: serde_json::Value,
    ) -> Self;

    /// Identity of this instance (instance ids included).
    fn window_type(&self) -> WindowType;

    /// Identity used by [`Self::open`]. Only meaningful for singletons;
    /// multi-instance windows override `open` to allocate an id.
    fn default_window_type() -> WindowType;

    /// State snapshot persisted into `WindowState.cache` on close.
    fn dump_cache(&mut self, _cx: &mut App) -> serde_json::Value {
        serde_json::Value::Null
    }

    /// Re-dump the dock layout (panels' `dump` payloads included) into the
    /// window's persisted cache. Dock layouts auto-persist on
    /// [`DockEvent::LayoutChanged`], but panel-level state changes (the chart
    /// panel's symbol/timeframe, for instance) fire no such event — panels
    /// call this through [`crate::windows::persist_window_dock`] after a
    /// change. No-op for windows without a dock.
    fn persist_dock_layout(&mut self, _cx: &mut Context<Self>) {}

    /// Open a window of this kind: activate the existing one when the kind
    /// is already open, otherwise restore remembered geometry and cache.
    fn open(cx: &mut App) {
        Self::open_instance(Self::default_window_type(), cx);
    }

    fn open_instance(window_type: WindowType, cx: &mut App) {
        if let Some(handle) = find_open_window(cx, window_type) {
            // Defer the activation: we may be running inside that very
            // window's update cycle (e.g. a click in its titlebar/panels),
            // and touching the window again now would fail with a spurious
            // "window not found". Deferred effects run after the flush.
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            });
            return;
        }

        let remembered = cx
            .global::<AppState>()
            .settings
            .window_state
            .iter()
            .find(|state| state.window_type == window_type)
            .cloned()
            .unwrap_or_else(|| WindowState::blank(window_type));

        let mut maximized = remembered.maximized;
        let remembered_bounds = if remembered.bounds.size.width <= px(0.) {
            maximized = false;
            default_bounds(cx, window_type)
        } else {
            remembered.bounds
        };
        // `display_id` is an HMONITOR handle value that goes stale across
        // sessions; re-resolve the monitor so gpui keeps the saved bounds
        // instead of discarding them for its centered default.
        let (display_id, remembered_bounds) =
            restore_display(cx, remembered.display_uuid.as_deref(), remembered_bounds);
        let cache = remembered.cache;

        let window_bounds = if maximized {
            gpui_kit::WindowBounds::Maximized(remembered_bounds)
        } else {
            gpui_kit::WindowBounds::Windowed(remembered_bounds)
        };

        let root_view = std::rc::Rc::new(std::cell::RefCell::new(None::<gpui_kit::Entity<Self>>));
        let root_view_slot = root_view.clone();
        let weak_view_slot: std::rc::Rc<std::cell::RefCell<Option<gpui_kit::WeakEntity<Self>>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));
        let weak_view_for_persist = weak_view_slot.clone();
        let opened = cx.open_window(
            window_options(window_type, window_bounds, display_id),
            move |window, cx| {
                let view = cx.new(|cx| Self::new(window, cx, window_type, cache));
                *root_view_slot.borrow_mut() = Some(view.clone());
                *weak_view_for_persist.borrow_mut() = Some(view.downgrade());
                // The window root must be gpui-kit's `Root`: its overlay
                // plugins (dialogs, notifications, tooltips) attach there,
                // and `WindowExt::open_dialog` resolves them through
                // `window.root::<Root>()` — a bare view root panics on use.
                cx.new(|cx| gpui_kit::component::Root::new(view, window, cx))
            },
        );
        match opened {
            Ok(handle) => {
                let root_view = root_view
                    .borrow_mut()
                    .take()
                    .expect("window root view was built");
                let weak_view = weak_view_slot
                    .borrow_mut()
                    .take()
                    .expect("window view was built");
                let persist: std::rc::Rc<dyn Fn(&mut App)> =
                    std::rc::Rc::new(move |cx: &mut App| {
                        let _ = weak_view.update(cx, |this, cx| this.persist_dock_layout(cx));
                    });
                register_open_window(cx, window_type, handle.into(), root_view.into(), persist);
                update_window_state(cx, window_type, |state| {
                    state.bounds = remembered_bounds;
                    state.maximized = maximized;
                    state.display_id = display_id;
                    state.show = true;
                });
            }
            Err(err) => {
                let window_label = window_type.title();
                tracing::error!(
                    "{}",
                    crate::tr!(
                        "app.log.window_open_failed",
                        window = window_label.clone(),
                        err = format!("{err:#}")
                    )
                );
                crate::utils::message::error(
                    crate::tr!("app.notify.window_open_failed", window = window_label),
                    format!("{err:#}"),
                    cx,
                );
            }
        }
    }

    /// Unified close: persist geometry, display and cache. Returning `true`
    /// lets the platform proceed with the close.
    fn close(window: &mut Window, cx: &mut App, this: WeakEntity<Self>) -> bool {
        if let Ok((window_type, cache)) =
            this.update(cx, |view, cx| (view.window_type(), view.dump_cache(cx)))
        {
            mark_window_closed(window, cx, window_type, cache);
        }
        true
    }

    fn render_titlebar_left(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
    }

    /// Content at the right end of the title bar, in front of the window
    /// controls. The area is not a drag region, so interactive elements
    /// (e.g. a settings entry) stay usable. Defaults to nothing.
    fn render_titlebar_right(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
    }

    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement;

    fn render_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_titlebar(window, cx))
            .child(self.render_content(window, cx))
    }

    fn render_titlebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_macos = cfg!(target_os = "macos");

        h_flex()
            .w_full()
            .h(TITLE_BAR_HEIGHT)
            .border_color(cx.theme().title_bar_border)
            .bg(cx.theme().title_bar)
            // Leftmost: window-specific content (e.g. the global search box);
            // not marked as drag so interactive elements stay usable
            .child(
                div()
                    .pl(TITLE_BAR_LEFT_PADDING)
                    .child(self.render_titlebar_left(window, cx)),
            )
            // Remaining blank: system drag region (double-click to maximize)
            .child(
                div()
                    .id("titlebar-drag")
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            // Right end: window-specific content (e.g. a settings entry),
            // vertically centered like the window controls beside it
            .child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .child(self.render_titlebar_right(window, cx)),
            )
            // Window control buttons (macOS relies on system traffic lights)
            .when(!is_macos, |bar| {
                bar.child(self.render_window_controls(window, cx))
            })
    }

    /// Window control buttons on the right of the title bar:
    /// minimize/maximize (restore)/close.
    ///
    /// On Windows they are delegated to the system natively via
    /// `WindowControlArea`; other non-macOS platforms fall back to `on_click`
    /// issuing window commands manually.
    fn render_window_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_windows = cfg!(target_os = "windows");
        let is_maximized = window.is_maximized();
        let foreground = cx.theme().foreground;
        let (normal_hover_bg, normal_hover_fg, normal_active_bg) = (
            cx.theme().secondary_hover,
            cx.theme().secondary_foreground,
            cx.theme().secondary_active,
        );
        let (close_hover_bg, close_hover_fg, close_active_bg) = (
            cx.theme().danger,
            cx.theme().danger_foreground,
            cx.theme().danger_active,
        );

        let control = |id: &'static str, icon: AppIcon, area: WindowControlArea, is_close: bool| {
            let (hover_bg, hover_fg, active_bg) = if is_close {
                (close_hover_bg, close_hover_fg, close_active_bg)
            } else {
                (normal_hover_bg, normal_hover_fg, normal_active_bg)
            };

            div()
                .id(id)
                .flex()
                .w(TITLE_BAR_HEIGHT)
                .h_full()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .text_color(foreground)
                .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
                .active(move |style| style.bg(active_bg))
                .when(is_windows, |this| this.window_control_area(area))
                .when(!is_windows, |this| {
                    this.on_click(move |_, window, _| match area {
                        WindowControlArea::Min => window.minimize_window(),
                        WindowControlArea::Max => window.zoom_window(),
                        _ => window.remove_window(),
                    })
                })
                .child(Icon::new(icon).small())
        };

        h_flex()
            .h_full()
            .flex_shrink_0()
            .child(control(
                "titlebar-minimize",
                AppIcon::WindowMinimize,
                WindowControlArea::Min,
                false,
            ))
            .child(control(
                "titlebar-maximize",
                if is_maximized {
                    AppIcon::WindowRestore
                } else {
                    AppIcon::WindowMaximize
                },
                WindowControlArea::Max,
                false,
            ))
            .child(control(
                "titlebar-close",
                AppIcon::WindowClose,
                WindowControlArea::Close,
                true,
            ))
    }
}
