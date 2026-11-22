//! Reusable UI pieces: the data tables, dialogs, title-bar menus and the
//! shared string picker, plus small element helpers.

pub mod datatable;
pub mod dialog;
pub mod kline;
pub mod menu;
pub mod picker;

use gpui_kit::{
    App, ClickEvent, SharedString, Window,
    component::{Icon, button::Button},
};

/// Compact icon button for combobox footers and toolbars (tooltip carries
/// the meaning; icon-only keeps narrow rows clean).
pub(crate) fn icon_button(
    id: &'static str,
    icon: impl Into<Icon>,
    tooltip: impl Into<SharedString>,
    disabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    use gpui_kit::component::{Disableable as _, Sizable as _, button::ButtonVariants as _};

    Button::new(id)
        .ghost()
        .small()
        .icon(icon)
        .tooltip(tooltip)
        .disabled(disabled)
        .on_click(on_click)
}
