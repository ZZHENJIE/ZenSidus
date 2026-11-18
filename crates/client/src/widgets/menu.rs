//! Title-bar dropdown menus (App / Panels / Windows). Each submodule pairs a
//! `menu()` element with an `actions()` installer that wires the menu's
//! actions onto a window root.

pub mod app;
pub mod panels;
pub mod windows;

use gpui_kit::{Div, Styled, base::h_flex};

/// The row the menus sit in, left of the title-bar drag region.
pub fn layout() -> Div {
    h_flex().gap_x_1()
}

/// `element.on_actions(menu::x::actions, cx)` reads better than nesting the
/// installer call around the element; that is all this does.
pub trait OnActionsExt<T: 'static>:
    Sized + gpui_kit::IntoElement + gpui_kit::InteractiveElement
{
    fn on_actions(
        self,
        f: impl FnOnce(Self, &mut gpui_kit::Context<T>) -> Self,
        cx: &mut gpui_kit::Context<T>,
    ) -> Self;
}

impl<T: 'static, E: gpui_kit::IntoElement + gpui_kit::InteractiveElement> OnActionsExt<T> for E {
    fn on_actions(
        self,
        f: impl FnOnce(E, &mut gpui_kit::Context<T>) -> E,
        cx: &mut gpui_kit::Context<T>,
    ) -> Self {
        f(self, cx)
    }
}
