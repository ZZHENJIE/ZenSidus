//! Zen Sidus design tokens → gpui-kit theme. The only layer allowed to hold
//! raw color values; everything else reads `cx.theme()` tokens.

use serde::{Deserialize, Serialize};

use gpui_kit::{
    App, Hsla,
    component::{Colorize as _, Theme as GpuiTheme, ThemeColor, ThemeMode},
    rgb, rgba,
};

const ZEN_BLACK: u32 = 0x0B0E14;
const BACKGROUND: u32 = 0x151922;
const TITLE_BAR: u32 = 0x0D1017;
const GLASS: u32 = 0x1D212A;
const GLASS_ELEVATED: u32 = 0x20242E;
const SIDUS_TEAL: u32 = 0x00D4B6;
const SIDUS_TEAL_HOVER: u32 = 0x00BFA4;
const SIDUS_TEAL_ACTIVE: u32 = 0x00AA93;
const DAWN_GOLD: u32 = 0xF5A623;
const TWILIGHT_PURPLE: u32 = 0x7B61FF;
const SIGNAL_RED: u32 = 0xFF4757;
const CLOUD_GRAY: u32 = 0x8B95A6;
const SOFT_WHITE: u32 = 0xE6E9EF;
const ALT_GREEN: u32 = 0x22C55E;

const BORDER: u32 = 0xFFFFFF1A;
const SURFACE: u32 = 0xFFFFFF0A;
const SURFACE_HOVER: u32 = 0xFFFFFF1A;
const SURFACE_ACTIVE: u32 = 0xFFFFFF26;
const TEAL_ACCENT: u32 = 0x00D4B61F;
const TEAL_SELECTION: u32 = 0x00D4B640;

/// User-overridable theme colors, persisted as `#RRGGBB[AA]` strings. Every
/// field is optional in the settings file (struct-level `serde(default)`), so
/// colors added after the first release keep old files loading.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Theme {
    pub primary: String,
    pub secondary: String,
    pub background: String,
    pub surface: String,
    pub text: String,
    pub border: String,
    pub error: String,
    pub success: String,
    pub warning: String,
    pub info: String,
    /// Chart marker color for rising quotes.
    pub bullish: String,
    /// Chart marker color for falling quotes.
    pub bearish: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            primary: hex6(SIDUS_TEAL),
            secondary: hex6(TWILIGHT_PURPLE),
            background: hex6(BACKGROUND),
            surface: hex6(GLASS),
            text: hex6(SOFT_WHITE),
            border: format!("#{:08X}", BORDER),
            error: hex6(SIGNAL_RED),
            success: hex6(ALT_GREEN),
            warning: hex6(DAWN_GOLD),
            info: hex6(TWILIGHT_PURPLE),
            bullish: hex6(SIGNAL_RED),
            bearish: hex6(ALT_GREEN),
        }
    }
}

fn hex6(color: u32) -> String {
    format!("#{:06X}", color & 0xFFFFFF)
}

/// Parse a `#RRGGBB` / `#RRGGBBAA` string (the `#` is optional); anything
/// else yields `fallback`.
pub(crate) fn parse_color(value: &str, fallback: Hsla) -> Hsla {
    let s = value.trim();
    let s = s.strip_prefix('#').unwrap_or(s);

    match s.len() {
        6 if s.bytes().all(|b| b.is_ascii_hexdigit()) => {
            let v = u32::from_str_radix(s, 16).unwrap();
            rgb(v).into()
        }
        8 if s.bytes().all(|b| b.is_ascii_hexdigit()) => {
            let v = u32::from_str_radix(s, 16).unwrap();
            rgba(v).into()
        }
        _ => fallback,
    }
}

impl Theme {
    /// Install the Zen Sidus design tokens into gpui-kit's theme, then layer
    /// the user's overrides on top. Call at startup and after every color
    /// change.
    pub fn load(&self, cx: &mut App) {
        GpuiTheme::change(ThemeMode::Dark, None, cx);

        GpuiTheme::update(cx, |theme| {
            let c: &mut ThemeColor = &mut theme.colors;
            apply_defaults(c);
            apply_overrides(c, self);
        });
    }
}

/// The fixed design tokens: the only place raw color values are allowed.
fn apply_defaults(c: &mut ThemeColor) {
    c.background = rgb(BACKGROUND).into();
    c.foreground = rgb(SOFT_WHITE).into();
    c.muted = rgb(GLASS).into();
    c.muted_foreground = rgb(CLOUD_GRAY).into();
    c.border = rgba(BORDER).into();
    c.input = rgba(BORDER).into();
    c.ring = rgb(SIDUS_TEAL).into();
    c.selection = rgba(TEAL_SELECTION).into();

    c.primary = rgb(SIDUS_TEAL).into();
    c.primary_foreground = rgb(ZEN_BLACK).into();
    c.primary_hover = rgb(SIDUS_TEAL_HOVER).into();
    c.primary_active = rgb(SIDUS_TEAL_ACTIVE).into();
    c.link = rgb(SIDUS_TEAL).into();
    c.link_hover = rgb(SIDUS_TEAL).into();

    c.secondary = rgba(SURFACE).into();
    c.secondary_foreground = rgb(SOFT_WHITE).into();
    c.secondary_hover = rgba(SURFACE_HOVER).into();
    c.secondary_active = rgba(SURFACE_ACTIVE).into();

    c.danger = rgb(SIGNAL_RED).into();
    c.danger_foreground = rgb(0xFFFFFF).into();
    c.danger_hover = rgb(0xFF5C6B).into();
    c.danger_active = rgb(0xF23F4F).into();
    c.warning = rgb(DAWN_GOLD).into();
    c.warning_foreground = rgb(ZEN_BLACK).into();
    c.info = rgb(TWILIGHT_PURPLE).into();
    c.info_foreground = rgb(0xFFFFFF).into();

    c.sidebar = rgb(BACKGROUND).into();
    c.sidebar_foreground = rgb(SOFT_WHITE).into();
    c.sidebar_border = rgba(BORDER).into();
    c.sidebar_accent = rgba(TEAL_ACCENT).into();
    c.sidebar_accent_foreground = rgb(SIDUS_TEAL).into();
    c.sidebar_primary = rgb(SIDUS_TEAL).into();
    c.sidebar_primary_foreground = rgb(ZEN_BLACK).into();

    c.title_bar = rgb(TITLE_BAR).into();
    c.title_bar_border = rgba(BORDER).into();
    c.status_bar = rgb(BACKGROUND).into();
    c.status_bar_border = rgba(BORDER).into();

    c.popover = rgb(GLASS_ELEVATED).into();
    c.popover_foreground = rgb(SOFT_WHITE).into();
    c.group_box = rgb(GLASS).into();
    c.list = rgb(BACKGROUND).into();
    c.list_hover = rgba(SURFACE).into();
    c.table = rgb(BACKGROUND).into();
    c.table_hover = rgba(SURFACE).into();

    c.chart_bullish = rgb(SIGNAL_RED).into();
    c.chart_bearish = rgb(ALT_GREEN).into();
    c.chart_grid = rgba(SURFACE_HOVER).into();
}

/// Derive every dependent token (hover/active ladders, sidebar accents,
/// muted text) from the user's base colors so one override restyles the
/// whole surface consistently.
fn apply_overrides(c: &mut ThemeColor, t: &Theme) {
    let primary = parse_color(&t.primary, c.primary);
    let background = parse_color(&t.background, c.background);
    let surface = parse_color(&t.surface, c.group_box);
    let text = parse_color(&t.text, c.foreground);
    let border = parse_color(&t.border, c.border);
    let error = parse_color(&t.error, c.danger);
    let success = parse_color(&t.success, rgb(ALT_GREEN).into());
    let warning = parse_color(&t.warning, c.warning);
    let info = parse_color(&t.info, c.info);

    c.primary = primary;
    c.primary_hover = primary.darken(0.08);
    c.primary_active = primary.darken(0.16);
    c.link = primary;
    c.link_hover = primary;
    c.ring = primary;
    c.sidebar_accent = primary.opacity(0.12);
    c.sidebar_accent_foreground = primary;
    c.sidebar_primary = primary;

    c.background = background;
    c.sidebar = background;
    c.list = background;
    c.table = background;
    c.status_bar = background;

    c.group_box = surface;
    c.muted = surface;
    // `secondary` is the tone of secondary buttons, drawn translucent over
    // the background (the ladder matches the previous surface-derived
    // rendering, so untouched configs look identical).
    let secondary = parse_color(&t.secondary, surface);
    c.secondary = secondary.opacity(0.4);
    c.secondary_hover = secondary.opacity(0.6);
    c.secondary_active = secondary.opacity(0.8);

    c.foreground = text;
    c.secondary_foreground = text;
    c.sidebar_foreground = text;
    c.popover_foreground = text;
    c.muted_foreground = text.opacity(0.6);

    c.border = border;
    c.input = border;
    c.sidebar_border = border;
    c.title_bar_border = border;
    c.status_bar_border = border;

    c.danger = error;
    c.warning = warning;
    c.info = info;
    c.success = success;

    c.chart_bullish = parse_color(&t.bullish, rgb(SIGNAL_RED).into());
    c.chart_bearish = parse_color(&t.bearish, rgb(ALT_GREEN).into());
}
