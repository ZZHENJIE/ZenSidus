//! Settings → About: the client and server versions. The server version is
//! only known while connected — it arrives with the Welcome frame
//! (`Welcome.version`) and lands on `AppState.server_version` via the hub's
//! version watcher.

use gpui_kit::{
    SharedString,
    component::{
        label::Label,
        setting::{SettingField, SettingGroup, SettingItem, SettingPage},
    },
};

use crate::tr;
use crate::utils::AppIcon;
use crate::utils::state::AppState;
use crate::windows::settings::SettingsWindow;

fn client_version_field() -> SettingField<SharedString> {
    SettingField::render(move |_, _, _| Label::new(concat!("v", env!("CARGO_PKG_VERSION"))))
}

fn server_version_field() -> SettingField<SharedString> {
    SettingField::render(move |_options, _window, cx| {
        let version = cx
            .global::<AppState>()
            .server_version
            .clone()
            .map(|v| SharedString::from(format!("v{v}")))
            .unwrap_or_else(|| tr!("app.settings.about_not_connected"));
        Label::new(version)
    })
}

impl SettingsWindow {
    pub(super) fn about_page(&self) -> SettingPage {
        super::settings_page("app.settings.about", AppIcon::Docs).group(
            SettingGroup::new()
                .item(SettingItem::new(
                    tr!("app.settings.about_client_version"),
                    client_version_field(),
                ))
                .item(SettingItem::new(
                    tr!("app.settings.about_server_version"),
                    server_version_field(),
                )),
        )
    }
}
