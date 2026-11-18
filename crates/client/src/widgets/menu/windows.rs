//! The "Windows" menu: open the small clock or a new custom panel window.

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
    windows::{AppWindow, CustomWindow, ETFLinkCalcWindow, SmallClockWindow},
};

pub fn menu() -> impl gpui_kit::IntoElement {
    Button::new("windows-menu")
        .ghost()
        .small()
        .label(tr!("app.menu.windows"))
        .dropdown_menu(|menu, _, _| {
            menu.menu_with_icon(
                tr!("app.menu.small_clock"),
                AppIcon::SmallClock,
                Box::new(actions::OpenSmallClock),
            )
            .menu_with_icon(
                tr!("app.menu.custom"),
                AppIcon::Custom,
                Box::new(actions::OpenCustom),
            )
            .menu_with_icon(
                tr!("app.menu.etf_linkcalc"),
                AppIcon::Tools,
                Box::new(actions::OpenETFLinkCalc),
            )
        })
}

pub fn actions<T: 'static, E: IntoElement + InteractiveElement>(
    div: E,
    cx: &mut gpui_kit::Context<T>,
) -> E {
    div.on_action(cx.listener(|_, &actions::OpenSmallClock, _, cx| SmallClockWindow::open(cx)))
        .on_action(cx.listener(|_, &actions::OpenCustom, _, cx| CustomWindow::open(cx)))
        .on_action(cx.listener(|_, &actions::OpenETFLinkCalc, _, cx| ETFLinkCalcWindow::open(cx)))
}
