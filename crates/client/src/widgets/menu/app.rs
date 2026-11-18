//! The "App" menu: settings window and documentation.

use gpui_kit::{
    InteractiveElement, IntoElement,
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        menu::DropdownMenu,
    },
};

use crate::tr;
use crate::{
    utils::{AppIcon, actions},
    windows::{AppWindow, SettingsWindow},
};

pub const APP_DOC_URL: &str = "https://zensidus.com/";

pub fn menu() -> impl gpui_kit::IntoElement {
    Button::new("app-menu")
        .ghost()
        .small()
        .label(tr!("app.menu.app"))
        .dropdown_menu(|menu, _, _| {
            menu.menu_with_icon(
                tr!("app.menu.settings"),
                AppIcon::Settings,
                Box::new(actions::OpenSettings),
            )
            .menu_with_icon(
                tr!("app.menu.docs"),
                AppIcon::Docs,
                Box::new(actions::OpenDocs),
            )
        })
}

pub fn actions<T: 'static, E: IntoElement + InteractiveElement>(
    div: E,
    cx: &mut gpui_kit::Context<T>,
) -> E {
    div.on_action(cx.listener(|_, &actions::OpenDocs, _, cx| cx.open_url(APP_DOC_URL)))
        .on_action(cx.listener(|_, &actions::OpenSettings, _, cx| SettingsWindow::open(cx)))
}
