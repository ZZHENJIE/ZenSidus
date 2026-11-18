//! Application actions: dispatched by the title-bar menus, handled by the
//! windows (`widgets::menu::*::actions`).

use gpui_kit::actions;
use serde::Deserialize;

// Application.
actions!(zen_sidus, [OpenSettings, OpenDocs]);

// Windows.
actions!(zen_sidus, [OpenSmallClock, OpenCustom, OpenETFLinkCalc]);

/// Open a fresh instance of the panel registered under `name` in the
/// dispatching window's center tab group. One action serves every panel —
/// the menu is generated from `panels::all()` — so adding a panel needs no
/// new action type.
#[derive(Clone, PartialEq, Eq, Deserialize, gpui_kit::Action)]
#[action(namespace = zen_sidus, no_json)]
pub struct OpenPanel {
    pub name: String,
}
