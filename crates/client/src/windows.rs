//! Window management: identities ([`WindowType`]), the open-window registry,
//! persisted geometry/layout ([`WindowState`] inside `settings.json`) and
//! startup restoration. Concrete windows live in the submodules and
//! implement [`AppWindow`] (and [`DockWindow`] when their content is a panel
//! layout).

pub mod app_window;
pub mod custom;
pub mod dock_window;
pub mod etf_linkcalc;
pub mod main;
pub mod settings;
pub mod small_clock;

pub use app_window::{AppWindow, titlebar_title};
pub use custom::CustomWindow;
pub use dock_window::{DockWindow, dump_dock};
pub use etf_linkcalc::ETFLinkCalcWindow;
pub use main::MainWindow;
pub use settings::SettingsWindow;
pub use small_clock::SmallClockWindow;

use std::collections::HashMap;

use gpui_kit::{
    AnyView, AnyWeakView, AnyWindowHandle, App, Bounds, Global, Pixels, SharedString,
    TitlebarOptions, Window, WindowKind, WindowOptions, point, px, size,
};
use serde::{Deserialize, Serialize};

use crate::tr;
use crate::utils::settings::update_settings;

/// Identity of a window.
///
/// [`WindowType::Main`]/[`WindowType::Settings`]/[`WindowType::SmallClock`]
/// are singletons: opening a window whose kind is already open just
/// activates the existing one. [`WindowType::Custom`] is multi-instance and
/// carries its instance id; new instances take the smallest id *not
/// currently open*, so closing one and re-opening brings back its
/// remembered geometry and layout.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum WindowType {
    Main,
    Settings,
    SmallClock,
    Custom(u32),
    ETFLinkCalc(u32),
}

impl WindowType {
    /// Whether two types belong to the same window family: same variant,
    /// any instance id. Used to enumerate all open instances of a kind.
    /// (Exact "is this window already open" checks use key equality on
    /// [`WindowType`] instead — see `find_open_window`.)
    pub(crate) fn same_kind(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Main, Self::Main)
                | (Self::Settings, Self::Settings)
                | (Self::SmallClock, Self::SmallClock)
                | (Self::Custom(_), Self::Custom(_))
                | (Self::ETFLinkCalc(_), Self::ETFLinkCalc(_))
        )
    }

    /// Instance id of a multi-instance window; singletons have none.
    pub fn instance_id(self) -> Option<u32> {
        match self {
            Self::Custom(id) => Some(id),
            Self::ETFLinkCalc(id) => Some(id),
            _ => None,
        }
    }

    /// Whether the window must always be restored at startup, no matter how
    /// it exited. The main window is the app's anchor and can never be
    /// "closed for good": its persisted `show` stays `true`.
    pub fn always_show(self) -> bool {
        matches!(self, Self::Main)
    }

    /// Restoration order at startup: main first so it starts focused.
    fn sort_key(self) -> u8 {
        match self {
            Self::Main => 0,
            Self::Settings => 1,
            Self::SmallClock => 2,
            Self::Custom(_) => 3,
            Self::ETFLinkCalc(_) => 4,
        }
    }

    /// Localized window title.
    pub fn title(self) -> SharedString {
        match self {
            Self::Main => tr!("app.windows.main.title"),
            Self::Settings => tr!("app.windows.settings.title"),
            Self::SmallClock => tr!("app.windows.small_clock.title"),
            Self::Custom(id) => format!("{} #{id}", tr!("app.windows.custom.title")).into(),
            Self::ETFLinkCalc(id) => {
                format!("{} #{id}", tr!("app.windows.etf_linkcalc.title")).into()
            }
        }
    }

    /// Element id of the window's dock area (only dock windows use it).
    pub fn dock_name(self) -> SharedString {
        match self {
            Self::Main => "main-dock".into(),
            Self::Settings => "settings-dock".into(),
            Self::SmallClock => "small-clock-dock".into(),
            Self::Custom(id) => format!("custom-{id}-dock").into(),
            Self::ETFLinkCalc(id) => format!("etf-linkcalc-{id}-dock").into(),
        }
    }
}

/// Persisted per-window record: geometry plus an opaque `cache` payload that
/// each window type defines itself (dock layouts live here, not in a
/// separate file).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WindowState {
    pub window_type: WindowType,
    /// Restore area, in the platform's own reopen semantics. When
    /// `maximized`, this is the restore (un-maximized) size.
    pub bounds: Bounds<Pixels>,
    /// Whether the window was maximized when persisted; restored as such.
    #[serde(default)]
    pub maximized: bool,
    pub display_id: Option<u64>,
    /// Stable identity of the display the window lived on (the gpui display
    /// uuid, derived from the monitor device name). `display_id` is an
    /// HMONITOR handle value — a session-scoped number that goes stale after
    /// a reboot or monitor topology change; this one survives and is what
    /// restore actually matches against.
    #[serde(default)]
    pub display_uuid: Option<String>,
    pub show: bool,
    pub cache: serde_json::Value,
}

impl WindowState {
    /// A record with nothing remembered yet: zero bounds (the window opens at
    /// its default size, centered) and not shown at startup.
    pub(crate) fn blank(window_type: WindowType) -> Self {
        Self {
            window_type,
            bounds: Bounds::default(),
            maximized: false,
            display_id: None,
            display_uuid: None,
            show: false,
            cache: serde_json::Value::Null,
        }
    }
}

/// The records of a fresh install: just the main window, shown.
pub(crate) fn window_state_default() -> Vec<WindowState> {
    vec![WindowState {
        show: true,
        ..WindowState::blank(WindowType::Main)
    }]
}

/// A currently open window: its handle (for activation / persistence) plus
/// a weak view reference for a read-only liveness check.
struct OpenWindow {
    handle: AnyWindowHandle,
    view: AnyWeakView,
}

/// Windows currently open, keyed by their exact [`WindowType`]. Powers the
/// "only one" rule: opening a window whose kind is already open just
/// activates the existing one instead.
#[derive(Default)]
struct WindowRegistry(HashMap<WindowType, OpenWindow>);

impl Global for WindowRegistry {}

fn registry_mut(cx: &mut App) -> &mut WindowRegistry {
    if !cx.has_global::<WindowRegistry>() {
        cx.set_global(WindowRegistry::default());
    }
    cx.global_mut::<WindowRegistry>()
}

fn register_open_window(
    cx: &mut App,
    window_type: WindowType,
    handle: AnyWindowHandle,
    view: AnyView,
) {
    registry_mut(cx).0.insert(
        window_type,
        OpenWindow {
            handle,
            view: view.downgrade(),
        },
    );
}

fn unregister_open_window(cx: &mut App, window_type: &WindowType) {
    if cx.has_global::<WindowRegistry>() {
        cx.global_mut::<WindowRegistry>().0.remove(window_type);
    }
}

/// Find the handle of the exact window type, if it is currently open.
///
/// Lookup is by exact key: singletons have unique variants, multi-instance
/// windows must match their instance id — otherwise opening Custom(2) while
/// Custom(1) is open would wrongly just focus Custom(1).
///
/// Liveness is checked via the weak view reference — a read-only probe that
/// never touches the window arena, so it also works while the window itself
/// is mid-update (e.g. from a click handler inside that very window).
pub(crate) fn find_open_window(cx: &App, window_type: WindowType) -> Option<AnyWindowHandle> {
    let registry = cx.try_global::<WindowRegistry>()?;
    let entry = registry.0.get(&window_type)?;
    if entry.view.upgrade().is_some() {
        Some(entry.handle)
    } else {
        None
    }
}

/// Smallest instance id of the given kind that is not currently open, so a
/// reused id brings back its remembered state.
fn allocate_instance_id(cx: &App, kind: WindowType) -> u32 {
    let Some(registry) = cx.try_global::<WindowRegistry>() else {
        return 1;
    };
    let mut open: Vec<u32> = registry
        .0
        .keys()
        .filter(|open| open.same_kind(kind))
        .filter_map(|open| open.instance_id())
        .collect();
    open.sort_unstable();

    let mut id = 1;
    for used in open {
        if used > id {
            break;
        }
        if used == id {
            id += 1;
        }
    }
    id
}

/// Create or update the persisted record of a window, then save settings.
pub(crate) fn update_window_state(
    cx: &mut App,
    window_type: WindowType,
    apply: impl FnOnce(&mut WindowState),
) {
    update_settings(cx, |settings| {
        let states = &mut settings.window_state;
        let index = match states
            .iter()
            .position(|state| state.window_type == window_type)
        {
            Some(index) => index,
            None => {
                states.push(WindowState::blank(window_type));
                states.len() - 1
            }
        };
        let state = &mut states[index];
        apply(state);
        // Hard rule: the main window is always restored at startup.
        if window_type.always_show() {
            state.show = true;
        }
    });
}

/// Persist the window's current geometry and display, plus whatever `extra`
/// sets on the record.
fn record_geometry(
    window: &Window,
    cx: &mut App,
    window_type: WindowType,
    extra: impl FnOnce(&mut WindowState),
) {
    let (bounds, maximized) = persisted_geometry(window);
    let display = window.display(cx);
    let display_id = display.as_ref().map(|display| display.id().into());
    let display_uuid = display
        .as_ref()
        .and_then(|display| display.uuid().ok())
        .map(|uuid| uuid.to_string());
    update_window_state(cx, window_type, |state| {
        state.bounds = bounds;
        state.maximized = maximized;
        state.display_id = display_id;
        state.display_uuid = display_uuid;
        extra(state);
    });
}

/// Record a window's geometry/cache on close and forget it is open.
fn mark_window_closed(
    window: &Window,
    cx: &mut App,
    window_type: WindowType,
    cache: serde_json::Value,
) {
    unregister_open_window(cx, &window_type);
    record_geometry(window, cx, window_type, |state| {
        state.cache = cache;
        state.show = window_type.always_show();
    });
}

/// Geometry in the platform's own reopen semantics.
///
/// `Window::bounds()` reports the client origin from `WM_MOVE`, while the
/// reopen path expands a saved client rect by *half* of each invisible-border
/// thickness (Windows: left/right ≈ symmetric, top = 0 / bottom = 7px). The
/// half-split makes that pair asymmetric vertically — each save/reopen cycle
/// shifted the window up by a few pixels. `Window::window_bounds()` is what
/// the platform itself intends to be reopened: it derives from
/// `GetWindowPlacement` through the exact same math, so saving it is drift-free.
fn persisted_geometry(window: &Window) -> (Bounds<Pixels>, bool) {
    match window.window_bounds() {
        gpui_kit::WindowBounds::Windowed(bounds) => (bounds, false),
        gpui_kit::WindowBounds::Maximized(bounds) => (bounds, true),
        gpui_kit::WindowBounds::Fullscreen(bounds) => (bounds, false),
    }
}

/// Re-resolve the target display for a window being restored from
/// `settings.json`.
///
/// [`WindowState::display_id`] is an HMONITOR handle value — a session-scoped
/// number that goes stale after a reboot or monitor topology change. When it
/// no longer resolves, gpui falls back to the primary display and then
/// discards the *entire* saved bounds (size included) unless their center
/// lands on that display, substituting its built-in centered default
/// instead. That is how a remembered 427×210 window could come back as a
/// 1536×1080 one. Hand the platform a display that provably contains the
/// saved bounds so its check passes and the remembered geometry survives:
///
/// 1. the stable uuid recorded in [`WindowState::display_uuid`],
/// 2. any current monitor containing the saved center (legacy records: no
///    uuid yet, but the topology is unchanged since the save),
/// 3. otherwise the primary display — keep the size, clamp the center into
///    it so the platform check still passes.
pub(crate) fn restore_display(
    cx: &App,
    display_uuid: Option<&str>,
    bounds: Bounds<Pixels>,
) -> (Option<u64>, Bounds<Pixels>) {
    if let Some(saved) = display_uuid
        && let Some(found) = cx.displays().into_iter().find(|display| {
            display.uuid().map(|uuid| uuid.to_string()).ok().as_deref() == Some(saved)
        })
    {
        return (Some(found.id().into()), bounds);
    }
    let center = bounds.center();
    if let Some(found) = cx
        .displays()
        .into_iter()
        .find(|display| display.bounds().contains(&center))
    {
        return (Some(found.id().into()), bounds);
    }
    match cx.primary_display() {
        Some(primary) => {
            let target = primary.bounds();
            let min_x = target.origin.x.as_f32();
            let min_y = target.origin.y.as_f32();
            let max_x = min_x + target.size.width.as_f32();
            let max_y = min_y + target.size.height.as_f32();
            let origin = point(
                px(center.x.as_f32().clamp(min_x, max_x) - bounds.size.width.as_f32() / 2.),
                px(center.y.as_f32().clamp(min_y, max_y) - bounds.size.height.as_f32() / 2.),
            );
            (
                Some(primary.id().into()),
                Bounds {
                    origin,
                    size: bounds.size,
                },
            )
        }
        None => (None, bounds),
    }
}

/// Persist the geometry of every currently open window without closing it.
/// Used on app quit, where windows are torn down without going through
/// [`AppWindow::close`]. Layout caches are already current (saved on every
/// layout change), so only bounds/display need a final write.
///
/// Must not run inside a window's own update (e.g. directly from one of its
/// click handlers): that window's handle cannot be re-entered there and its
/// geometry would be the one skipped. Callers in that position `cx.defer`
/// the call.
pub(crate) fn persist_open_windows(cx: &mut App) {
    let open: Vec<(WindowType, AnyWindowHandle)> = cx
        .try_global::<WindowRegistry>()
        .map(|registry| {
            registry
                .0
                .iter()
                .filter(|(_, open)| open.view.upgrade().is_some())
                .map(|(window_type, open)| (*window_type, open.handle))
                .collect()
        })
        .unwrap_or_default();
    for (window_type, handle) in open {
        if let Err(err) = handle.update(cx, |_, window, app| {
            record_geometry(window, app, window_type, |_| {});
        }) {
            tracing::warn!(
                "{}",
                tr!(
                    "app.log.geometry_not_persisted",
                    window = format!("{window_type:?}"),
                    err = format!("{err:#}")
                )
            );
        }
    }
    // The geometry mutations above go through the debounced settings save;
    // flush synchronously so quitting right after cannot lose them.
    crate::utils::settings::flush_settings(cx);
}

/// Default geometry for a window that has no remembered bounds yet.
fn default_bounds(cx: &App, window_type: WindowType) -> Bounds<Pixels> {
    let (width, height) = match window_type {
        WindowType::Main => (1280., 800.),
        WindowType::Settings | WindowType::Custom(_) => (960., 640.),
        WindowType::SmallClock => (300., 80.),
        WindowType::ETFLinkCalc(_) => (480., 300.),
    };
    app_window::centered_bounds_size(cx, width, height)
}

fn window_options(
    window_type: WindowType,
    window_bounds: gpui_kit::WindowBounds,
    display_id: Option<u64>,
) -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(window_type.title()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(9.0), px(9.0))),
        }),
        display_id: display_id.map(|id| id.into()),
        kind: match window_type {
            WindowType::SmallClock => WindowKind::PopUp,
            _ => WindowKind::Normal,
        },
        window_bounds: Some(window_bounds),
        window_min_size: match window_type {
            WindowType::SmallClock => Some(size(px(250.0), px(60.0))),
            _ => Some(size(px(280.0), px(210.0))),
        },
        ..Default::default()
    }
}

/// Restore every window marked visible in the persisted state. `AppState`
/// must already be registered as a global when this runs.
pub fn init(cx: &mut App, states: &[WindowState]) {
    registry_mut(cx);

    let mut types: Vec<WindowType> = states
        .iter()
        .filter(|state| state.show)
        .map(|state| state.window_type)
        .collect();
    types.sort_by_key(|window_type| window_type.sort_key());

    for window_type in types {
        match window_type {
            WindowType::Main => MainWindow::open_instance(window_type, cx),
            WindowType::Settings => SettingsWindow::open_instance(window_type, cx),
            WindowType::SmallClock => SmallClockWindow::open_instance(window_type, cx),
            WindowType::Custom(_) => CustomWindow::open_instance(window_type, cx),
            WindowType::ETFLinkCalc(_) => ETFLinkCalcWindow::open_instance(window_type, cx),
        }
    }
}

/// Wire the platform close request to [`AppWindow::close`]. Call from each
/// window's `new`.
pub fn hook_should_close<W: AppWindow>(window: &mut Window, cx: &mut gpui_kit::Context<W>) {
    let weak = cx.weak_entity();
    window.on_window_should_close(cx, move |window, cx| W::close(window, cx, weak.clone()));
}
