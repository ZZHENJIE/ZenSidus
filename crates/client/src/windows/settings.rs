//! The Settings window: a gpui-kit `Settings` shell with one page per
//! submodule (general / alpaca / finviz / logs). Color pickers are created
//! here and seeded from the persisted theme; a commit persists the hex and
//! re-applies the theme tokens.

mod server;

mod general;
mod logs;
mod market_screener;
mod small_clock;

use gpui_kit::{
    AppContext as _, Context, Entity, Hsla, IntoElement, ParentElement as _, Render, Rgba,
    SharedString, Styled as _, Window,
    component::{
        color_picker::ColorPickerEvent,
        color_picker::ColorPickerState,
        setting::{SettingField, SettingPage, Settings},
    },
    div,
};

use crate::utils::state::{HubEvent, AppState};
use crate::utils::{AppIcon, Theme, settings, theme};
use crate::windows::{AppWindow, WindowType, hook_should_close};

use self::logs::LogsState;

/// The nine base theme tones of the "Colors" group, in display order. Each
/// maps one picker to one `Theme` config field.
#[derive(Clone, Copy)]
pub(super) enum ToneField {
    Background,
    Surface,
    Text,
    Border,
    Secondary,
    Success,
    Warning,
    Info,
    Error,
}

const TONE_FIELDS: [ToneField; 9] = [
    ToneField::Background,
    ToneField::Surface,
    ToneField::Text,
    ToneField::Border,
    ToneField::Secondary,
    ToneField::Success,
    ToneField::Warning,
    ToneField::Info,
    ToneField::Error,
];

impl ToneField {
    fn label_key(self) -> &'static str {
        match self {
            ToneField::Background => "app.settings.color_background",
            ToneField::Surface => "app.settings.color_surface",
            ToneField::Text => "app.settings.color_text",
            ToneField::Border => "app.settings.color_border",
            ToneField::Secondary => "app.settings.color_secondary",
            ToneField::Success => "app.settings.color_success",
            ToneField::Warning => "app.settings.color_warning",
            ToneField::Info => "app.settings.color_info",
            ToneField::Error => "app.settings.color_error",
        }
    }

    fn desc_key(self) -> &'static str {
        match self {
            ToneField::Background => "app.settings.color_background_desc",
            ToneField::Surface => "app.settings.color_surface_desc",
            ToneField::Text => "app.settings.color_text_desc",
            ToneField::Border => "app.settings.color_border_desc",
            ToneField::Secondary => "app.settings.color_secondary_desc",
            ToneField::Success => "app.settings.color_success_desc",
            ToneField::Warning => "app.settings.color_warning_desc",
            ToneField::Info => "app.settings.color_info_desc",
            ToneField::Error => "app.settings.color_error_desc",
        }
    }

    /// The hex currently persisted for this tone.
    fn seed(self, theme: &Theme) -> String {
        match self {
            ToneField::Background => theme.background.clone(),
            ToneField::Surface => theme.surface.clone(),
            ToneField::Text => theme.text.clone(),
            ToneField::Border => theme.border.clone(),
            ToneField::Secondary => theme.secondary.clone(),
            ToneField::Success => theme.success.clone(),
            ToneField::Warning => theme.warning.clone(),
            ToneField::Info => theme.info.clone(),
            ToneField::Error => theme.error.clone(),
        }
    }

    fn apply(self, theme: &mut Theme, hex: String) {
        match self {
            ToneField::Background => theme.background = hex,
            ToneField::Surface => theme.surface = hex,
            ToneField::Text => theme.text = hex,
            ToneField::Border => theme.border = hex,
            ToneField::Secondary => theme.secondary = hex,
            ToneField::Success => theme.success = hex,
            ToneField::Warning => theme.warning = hex,
            ToneField::Info => theme.info = hex,
            ToneField::Error => theme.error = hex,
        }
    }
}

/// Application settings. A singleton: opening it while it is open just
/// activates the existing window.
pub struct SettingsWindow {
    primary_picker: Entity<ColorPickerState>,
    bullish_picker: Entity<ColorPickerState>,
    bearish_picker: Entity<ColorPickerState>,
    tone_pickers: Vec<(ToneField, Entity<ColorPickerState>)>,
    logs: Entity<LogsState>,
}

impl AppWindow for SettingsWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        _window_type: WindowType,
        _cache: serde_json::Value,
    ) -> Self {
        hook_should_close(window, cx);

        // Seed each picker with its currently persisted color.
        let theme_snapshot = cx.global::<AppState>().settings.theme.clone();
        let fallback = gpui_kit::hsla(0., 0., 0.5, 1.);
        let primary_picker = make_picker(
            window,
            cx,
            theme::parse_color(&theme_snapshot.primary, fallback),
        );
        let bullish_picker = make_picker(
            window,
            cx,
            theme::parse_color(&theme_snapshot.bullish, fallback),
        );
        let bearish_picker = make_picker(
            window,
            cx,
            theme::parse_color(&theme_snapshot.bearish, fallback),
        );
        let tone_pickers = TONE_FIELDS
            .map(|field| {
                let hex = field.seed(&theme_snapshot);
                (
                    field,
                    make_picker(window, cx, theme::parse_color(&hex, fallback)),
                )
            })
            .to_vec();

        let logs = cx.new(|_| LogsState::new());

        // Every settings mutation goes through `global_mut::<AppState>`, so
        // observing the global refreshes this window (and every other open
        // window) on any change — including language switches.
        cx.observe_global::<AppState>(|_, cx| cx.notify()).detach();

        // Connection status changes come from the hub's background thread.
        let hub = cx.global::<AppState>().hub.clone();
        cx.subscribe(&hub, |_, _, _: &HubEvent, cx| {
            cx.notify();
        })
        .detach();

        // Picker commits: persist the hex and re-apply the theme tokens.
        subscribe_color(cx, &primary_picker, |theme, v| theme.primary = v);
        subscribe_color(cx, &bullish_picker, |theme, v| theme.bullish = v);
        subscribe_color(cx, &bearish_picker, |theme, v| theme.bearish = v);
        for (field, picker) in tone_pickers.iter() {
            let field = *field;
            subscribe_color(cx, picker, move |theme, v| field.apply(theme, v));
        }

        cx.observe(&logs, |_, _, cx| cx.notify()).detach();

        Self {
            primary_picker,
            bullish_picker,
            bearish_picker,
            tone_pickers,
            logs,
        }
    }

    fn window_type(&self) -> WindowType {
        WindowType::Settings
    }

    fn default_window_type() -> WindowType {
        WindowType::Settings
    }

    fn render_titlebar_left(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        super::titlebar_title(WindowType::Settings.title())
    }

    fn render_content(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.logs.update(cx, |logs, _| logs.prepare());

        div().flex_1().min_h_0().overflow_hidden().child(
            Settings::new("settings")
                .page(self.general_page())
                .page(self.server_page())

                .page(self.market_screener_page())
                .page(self.small_clock_page())
                .page(self.logs_page()),
        )
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_window(window, cx)
    }
}

impl SettingsWindow {
    /// The base-tone pickers of the "Colors" group, in display order.
    pub(super) fn tone_pickers(&self) -> &[(ToneField, Entity<ColorPickerState>)] {
        &self.tone_pickers
    }
}

/// An input field bound to one `String` setting: `get` reads it for display,
/// `set` writes the edited value back (persisted through `update_settings`).
pub(super) fn text_setting(
    get: fn(&settings::Settings) -> &String,
    set: fn(&mut settings::Settings, String),
) -> SettingField<SharedString> {
    SettingField::input(
        move |cx| get(&cx.global::<AppState>().settings).clone().into(),
        move |value, cx| settings::update_settings(cx, |s| set(s, value.to_string())),
    )
}

/// One settings page with the common shell: translated title, closed by
/// default in the nav, page icon. Pages chain `.description(...)`/`.group(...)`
/// on top.
pub(super) fn settings_page(title_key: &'static str, icon: AppIcon) -> SettingPage {
    SettingPage::new(crate::tr!(title_key))
        .default_open(false)
        .icon(icon)
}

/// Wire one picker so a commit persists into the theme field `apply` targets
/// and re-applies the theme tokens.
fn subscribe_color(
    cx: &mut Context<SettingsWindow>,
    picker: &Entity<ColorPickerState>,
    apply: impl Fn(&mut Theme, String) + 'static,
) {
    cx.subscribe(picker, move |_, _, event: &ColorPickerEvent, cx| {
        if let ColorPickerEvent::Change(Some(color)) = event {
            let hex = hex_string(*color);
            settings::update_settings(cx, |s| apply(&mut s.theme, hex));
            cx.global::<AppState>().settings.theme.clone().load(cx);
        }
    })
    .detach();
}

fn make_picker(
    window: &mut Window,
    cx: &mut Context<SettingsWindow>,
    color: Hsla,
) -> Entity<ColorPickerState> {
    cx.new(|cx| {
        let mut state = ColorPickerState::new(window, cx);
        state.set_value(color, window, cx);
        state
    })
}

/// Persisted theme colors are `#RRGGBB[AA]` strings; the pickers work in
/// `Hsla`. Round-trip through `Rgba` with alpha appended only when present.
fn hex_string(color: Hsla) -> String {
    let rgba = Rgba::from(color);
    let r = (rgba.r * 255.).round() as u32;
    let g = (rgba.g * 255.).round() as u32;
    let b = (rgba.b * 255.).round() as u32;
    let a = (rgba.a * 255.).round() as u32;
    if a == 255 {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    }
}
