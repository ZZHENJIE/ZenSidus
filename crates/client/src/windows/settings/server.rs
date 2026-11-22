//! Settings → Server: the server connection (the app's only outbound
//! data link) and the live connection status with connect/disconnect
//! actions. Connecting asks for the auth code in a dialog — it is never
//! persisted; address/TLS edits apply on the next connect.

use gpui_kit::{
    IntoElement as _, ParentElement as _, Styled as _,
    base::h_flex,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        setting::{SettingField, SettingGroup, SettingItem, SettingPage},
    },
    div, px,
};

use crate::tr;
use crate::utils::state::{AppState, ServerStatus};
use crate::utils::{AppIcon, settings};
use crate::widgets::dialog::open_auth_code_dialog;
use crate::windows::settings::{SettingsWindow, text_setting};

/// Status dot + label, reflecting the hub live (the window subscribes to
/// `HubEvent::StatusChanged`). A `Failed` status appends the reason so
/// the failure is explainable without opening the logs.
fn server_status_item() -> SettingItem {
    SettingItem::render(move |_, _, cx| {
        let status: ServerStatus = cx.global::<AppState>().hub.read(cx).status().clone();
        h_flex()
            .gap_2()
            .child(div().size(px(8.)).rounded_full().bg(status.color(cx)))
            .child(div().child(status.label()))
            .children(
                status
                    .reason()
                    .filter(|reason| !reason.is_empty())
                    .map(|reason| {
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .text_sm()
                            .child(reason.clone())
                    }),
            )
            .into_any_element()
    })
    .keywords(["server status"])
}

fn server_actions_item() -> SettingItem {
    SettingItem::render(move |_, _, cx| {
        let hub = cx.global::<AppState>().hub.clone();
        let connecting = matches!(hub.read(cx).status(), ServerStatus::Connecting);
        let up = hub.read(cx).connected();

        let connect = {
            let hub = hub.clone();
            Button::new("server-connect")
                .primary()
                .small()
                .label(tr!("app.settings.connect"))
                .disabled(connecting || up)
                .on_click(move |_, window, cx| {
                    // The auth code is typed per connect in dialog; the hub
                    // re-reads address/TLS from the settings itself.
                    open_auth_code_dialog(window, cx, {
                        let hub = hub.clone();
                        move |code, cx| hub.update(cx, |hub, cx| hub.connect(code.to_string(), cx))
                    });
                })
        };
        let disconnect = Button::new("server-disconnect")
            .outline()
            .small()
            .label(tr!("app.settings.disconnect"))
            .disabled(!up && !connecting)
            .on_click({
                let hub = hub.clone();
                move |_, _, cx| hub.update(cx, |hub, cx| hub.disconnect(cx))
            });

        h_flex().gap_2().child(connect).child(disconnect)
    })
    .keywords(["server connect disconnect"])
}

impl SettingsWindow {
    pub(super) fn server_page(&self) -> SettingPage {
        super::settings_page("app.settings.server", AppIcon::Market)
            .description(tr!("app.settings.server_desc"))
            .group(
                SettingGroup::new()
                    .title(tr!("app.settings.connection"))
                    .item(
                        SettingItem::new(
                            tr!("app.settings.server_addr"),
                            text_setting(|s| &s.server.addr, |s, v| s.server.addr = v),
                        )
                        .description(tr!("app.settings.server_addr_desc")),
                    )
                    .item(
                        SettingItem::new(
                            tr!("app.settings.server_tls"),
                            SettingField::switch(
                                |cx| cx.global::<AppState>().settings.server.tls,
                                |value, cx| {
                                    settings::update_settings(cx, |s| s.server.tls = value);
                                },
                            ),
                        )
                        .description(tr!("app.settings.server_tls_desc")),
                    )
                    .item(
                        SettingItem::new(
                            tr!("app.settings.server_tls_cert"),
                            text_setting(|s| &s.server.tls_cert, |s, v| s.server.tls_cert = v),
                        )
                        .description(tr!("app.settings.server_tls_cert_desc")),
                    )
                    .item(server_status_item())
                    .item(server_actions_item()),
            )
    }
}
